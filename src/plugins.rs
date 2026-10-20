//! Sandboxed plugin packages, capability grants and the off-thread script host.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};

use rhai::{Dynamic, Engine, Scope};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub const API_VERSION: u32 = 1;
const MAX_PACKAGE: usize = 1024 * 1024;
const MAX_SOURCE: usize = 128 * 1024;
const MAX_MANIFEST: usize = 32 * 1024;
const MAX_OUTPUT: usize = 128 * 1024;
const MAX_PLUGINS: usize = 32;
// All plugins may use all 32 text settings, including JSON escaping overhead.
const MAX_CONFIG: usize = 32 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    PlaybackRead,
    PlaybackControl,
    TracksWrite,
    NetworkProxy,
    UiWrite,
    BehaviorWrite,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum LocalizedText {
    Text(String),
    Translations(BTreeMap<String, String>),
}

impl LocalizedText {
    pub fn resolve(&self, locale: &str) -> &str {
        match self {
            Self::Text(text) => text,
            Self::Translations(texts) => texts
                .get(locale)
                .or_else(|| texts.get(locale.split(['-', '_']).next().unwrap_or(locale)))
                .or_else(|| texts.get("en"))
                .map_or("", String::as_str),
        }
    }

    fn validate(&self) -> Result<(), String> {
        let valid = |text: &str| !text.trim().is_empty() && text.len() <= 1024;
        match self {
            Self::Text(text) if valid(text) => Ok(()),
            Self::Translations(texts)
                if texts.len() <= 64
                    && texts.get("en").is_some_and(|text| valid(text))
                    && texts
                        .iter()
                        .all(|(key, text)| key.len() <= 32 && valid(text)) =>
            {
                Ok(())
            }
            _ => Err("Invalid localized text".into()),
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SettingKind {
    Bool,
    Text,
    Number,
    Select,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingDefinition {
    pub label: LocalizedText,
    pub kind: SettingKind,
    pub default: Value,
    #[serde(default)]
    pub min: Option<f64>,
    #[serde(default)]
    pub max: Option<f64>,
    #[serde(default)]
    pub options: Vec<Value>,
}

impl SettingDefinition {
    pub fn accepts(&self, value: &Value) -> bool {
        match self.kind {
            SettingKind::Bool => value.is_boolean(),
            SettingKind::Text => value.as_str().is_some_and(|text| text.len() <= 4096),
            SettingKind::Number => value.as_f64().is_some_and(|number| {
                number.is_finite()
                    && self.min.is_none_or(|min| number >= min)
                    && self.max.is_none_or(|max| number <= max)
            }),
            SettingKind::Select => self.options.contains(value),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub id: String,
    pub name: LocalizedText,
    pub version: String,
    pub api: u32,
    #[serde(default = "default_entry")]
    pub entry: String,
    #[serde(default)]
    pub permissions: BTreeSet<Permission>,
    #[serde(default)]
    pub settings: BTreeMap<String, SettingDefinition>,
}

fn default_entry() -> String {
    "main.rhai".into()
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && id.as_bytes()[0].is_ascii_lowercase()
}

impl Manifest {
    fn validate(&self) -> Result<(), String> {
        if !valid_id(&self.id)
            || self.api != API_VERSION
            || self.version.is_empty()
            || self.version.len() > 64
        {
            return Err("Invalid plugin identity or API version".into());
        }
        self.name.validate()?;
        if self.entry != "main.rhai" || self.settings.len() > 32 {
            return Err("Invalid plugin entry or settings".into());
        }
        for (key, setting) in &self.settings {
            setting.label.validate()?;
            if !valid_id(key)
                || !setting.accepts(&setting.default)
                || setting.options.len() > 64
                || serde_json::to_vec(&setting.options)
                    .map_err(|_| "Invalid setting")?
                    .len()
                    > 8192
                || setting
                    .min
                    .zip(setting.max)
                    .is_some_and(|(min, max)| min > max)
            {
                return Err("Invalid plugin setting".into());
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct Package {
    pub manifest: Manifest,
    pub source: Arc<str>,
    pub digest: String,
    archive: Arc<[u8]>,
}

impl Package {
    /// Parse without extracting anything to disk, including unused archive entries.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, String> {
        if bytes.len() > MAX_PACKAGE {
            return Err("Plugin package is too large".into());
        }
        let mut zip =
            zip::ZipArchive::new(Cursor::new(&bytes)).map_err(|_| "Invalid plugin ZIP")?;
        if zip.len() > 32 {
            return Err("Too many plugin files".into());
        }
        zip.validate_header_consistency()
            .map_err(|_| "Inconsistent plugin ZIP headers")?;
        let mut names = BTreeSet::new();
        let mut files = BTreeMap::new();
        let mut expanded = 0_u64;
        for index in 0..zip.len() {
            let mut file = zip
                .by_index(index)
                .map_err(|_| "Invalid plugin ZIP entry")?;
            let name = file
                .name()
                .map_err(|_| "Invalid plugin filename")?
                .into_owned();
            if name.is_empty()
                || name.len() > 128
                || name.contains(['\\', ':', '\0'])
                || name.starts_with('/')
                || name
                    .split('/')
                    .any(|part| part == ".." || part == "." || part.is_empty())
                || !names.insert(name.clone())
                || file
                    .unix_mode()
                    .is_some_and(|mode| mode & 0o170000 == 0o120000)
            {
                return Err("Unsafe or duplicate plugin filename".into());
            }
            expanded = expanded
                .checked_add(file.size())
                .ok_or("Plugin package is too large")?;
            if expanded > MAX_PACKAGE as u64 {
                return Err("Expanded plugin package is too large".into());
            }
            let limit = match name.as_str() {
                "manifest.json" => MAX_MANIFEST,
                "main.rhai" => MAX_SOURCE,
                _ => MAX_PACKAGE,
            };
            let mut content = Vec::new();
            file.by_ref()
                .take(limit as u64 + 1)
                .read_to_end(&mut content)
                .map_err(|_| "Unable to read plugin file")?;
            if content.len() > limit {
                return Err("Plugin file is too large".into());
            }
            files.insert(name, content);
        }
        let manifest: Manifest =
            serde_json::from_slice(files.get("manifest.json").ok_or("Missing manifest.json")?)
                .map_err(|_| "Invalid plugin manifest")?;
        manifest.validate()?;
        let source = String::from_utf8(files.remove(&manifest.entry).ok_or("Missing main.rhai")?)
            .map_err(|_| "Plugin source must be UTF-8")?;
        // Compile only: installing never executes a plugin, even its top-level code.
        engine()
            .compile(&source)
            .map_err(|_| "Invalid plugin script")?;
        let digest = format!("{:x}", Sha256::digest(&bytes));
        drop(zip);
        Ok(Self {
            manifest,
            source: source.into(),
            digest,
            archive: bytes.into(),
        })
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PluginConfig {
    pub digest: String,
    pub enabled: bool,
    pub grants: BTreeSet<Permission>,
    pub settings: BTreeMap<String, Value>,
    pub revision: u64,
}

#[derive(Clone, Debug)]
pub struct PluginRecord {
    pub package: Package,
    pub config: PluginConfig,
    pub error: Option<String>,
}

impl PluginRecord {
    pub fn permits(&self, permission: Permission) -> bool {
        self.config.enabled
            && self.config.digest == self.package.digest
            && self.package.manifest.permissions.contains(&permission)
            && self.config.grants.contains(&permission)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum PluginCommand {
    Play { uri: String },
    Pause,
    Resume,
    Next,
    Previous,
    Seek { position_ms: u32 },
    Volume { value: u8 },
    Shuffle { enabled: bool },
    Repeat { mode: String },
    Queue { uri: String },
    QueueMany { uris: Vec<String> },
    PlayNext { uris: Vec<String> },
    ClearQueue,
    OpenPage { page: String },
    Search { query: String },
    Emit { event: String },
    Open { uri: String },
}

impl PluginCommand {
    pub fn validate(&self) -> bool {
        match self {
            Self::QueueMany { uris } | Self::PlayNext { uris } => {
                !uris.is_empty()
                    && uris.len() <= 32
                    && uris
                        .iter()
                        .all(|uri| Self::Queue { uri: uri.clone() }.validate())
            }
            Self::OpenPage { page } => matches!(
                page.as_str(),
                "home"
                    | "top-songs"
                    | "new-releases"
                    | "search"
                    | "liked"
                    | "albums"
                    | "artists"
                    | "podcasts"
                    | "episodes"
                    | "queue"
                    | "settings"
            ),
            Self::Search { query } => !query.is_empty() && query.len() <= 1024,
            Self::Emit { event } => !event.is_empty() && event.len() <= 128,
            Self::Queue { uri } => {
                (uri.starts_with("spotify:track:") || uri.starts_with("spotify:episode:"))
                    && Self::Play { uri: uri.clone() }.validate()
            }
            Self::Play { uri } | Self::Open { uri } => {
                let mut parts = uri.split(':');
                parts.next() == Some("spotify")
                    && matches!(
                        parts.next(),
                        Some("track" | "episode" | "album" | "playlist" | "artist" | "show")
                    )
                    && parts.next().is_some_and(|id| {
                        !id.is_empty()
                            && id.len() <= 64
                            && id.bytes().all(|byte| byte.is_ascii_alphanumeric())
                    })
                    && parts.next().is_none()
            }
            Self::Volume { value } => *value <= 100,
            Self::Repeat { mode } => matches!(mode.as_str(), "off" | "track" | "context"),
            _ => true,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TrackPatch {
    pub name: Option<String>,
    pub artists: Option<Vec<String>>,
    pub album: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum UiNode {
    Label {
        text: LocalizedText,
    },
    Button {
        text: LocalizedText,
        command: PluginCommand,
    },
    Row {
        children: Vec<UiNode>,
    },
    Column {
        children: Vec<UiNode>,
    },
}

impl UiNode {
    fn validate(&self, depth: usize, count: &mut usize) -> bool {
        *count += 1;
        if depth > 8 || *count > 128 {
            return false;
        }
        match self {
            Self::Label { text } => text.validate().is_ok(),
            Self::Button { text, command } => text.validate().is_ok() && command.validate(),
            Self::Row { children } | Self::Column { children } => {
                children.iter().all(|node| node.validate(depth + 1, count))
            }
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProxyPatch {
    pub mode: String,
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub port: String,
}

impl ProxyPatch {
    fn validate(&self) -> bool {
        let mode = match self.mode.as_str() {
            "off" => crate::settings::ProxyMode::Off,
            "system" => crate::settings::ProxyMode::System,
            "http" => crate::settings::ProxyMode::Http,
            "socks" => crate::settings::ProxyMode::Socks,
            _ => return false,
        };
        self.host.len() <= 253
            && self.port.len() <= 5
            && crate::settings::Settings {
                proxy_mode: mode,
                proxy_host: self.host.clone(),
                proxy_port: self.port.clone(),
                ..Default::default()
            }
            .proxy_config()
            .is_ok()
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Output {
    pub state: Value,
    pub commands: Vec<PluginCommand>,
    pub track_overrides: BTreeMap<String, TrackPatch>,
    pub ui: Vec<UiNode>,
    pub ui_slots: BTreeMap<String, Vec<UiNode>>,
    pub ui_replace: BTreeSet<String>,
    pub block_actions: BTreeSet<String>,
    pub action_rules: BTreeMap<String, PluginCommand>,
    pub proxy: Option<ProxyPatch>,
}

impl Output {
    fn validate(&self) -> Result<(), String> {
        let mut count = 0;
        if self
            .commands
            .iter()
            .map(|command| match command {
                PluginCommand::QueueMany { uris } | PluginCommand::PlayNext { uris } => uris.len(),
                _ => 1,
            })
            .sum::<usize>()
            > 32
            || self.track_overrides.len() > 512
            || self.action_rules.len() > 16
            || !self
                .commands
                .iter()
                .all(|command| !matches!(command, PluginCommand::Emit { .. }) && command.validate())
            || !self.ui.iter().all(|node| node.validate(0, &mut count))
            || !self.ui_slots.iter().all(|(slot, nodes)| {
                matches!(
                    slot.as_str(),
                    "page_start" | "page_end" | "sidebar" | "player_bar" | "topbar" | "settings"
                ) && nodes.iter().all(|node| node.validate(0, &mut count))
            })
            || !self.ui_replace.iter().all(|slot| slot == "page")
            || !self.block_actions.iter().all(|key| {
                matches!(
                    key.as_str(),
                    "toggle_play"
                        | "next"
                        | "previous"
                        | "seek"
                        | "volume"
                        | "shuffle"
                        | "repeat"
                        | "queue"
                        | "play"
                )
            })
            || !self.action_rules.iter().all(|(key, command)| {
                matches!(
                    key.as_str(),
                    "toggle_play"
                        | "next"
                        | "previous"
                        | "seek"
                        | "volume"
                        | "shuffle"
                        | "repeat"
                        | "queue"
                        | "play"
                ) && !matches!(command, PluginCommand::Emit { .. })
                    && command.validate()
            })
            || !self.track_overrides.iter().all(|(uri, patch)| {
                PluginCommand::Play { uri: uri.clone() }.validate()
                    && patch.name.as_ref().is_none_or(|text| text.len() <= 1024)
                    && patch.album.as_ref().is_none_or(|text| text.len() <= 1024)
                    && patch.artists.as_ref().is_none_or(|artists| {
                        artists.len() <= 32 && artists.iter().all(|text| text.len() <= 1024)
                    })
            })
            || self.proxy.as_ref().is_some_and(|proxy| !proxy.validate())
        {
            return Err("Invalid plugin output".into());
        }
        if serde_json::to_vec(self)
            .map_err(|_| "Invalid plugin output")?
            .len()
            > MAX_OUTPUT
        {
            return Err("Plugin output is too large".into());
        }
        Ok(())
    }

    fn restrict(&mut self, config: &PluginConfig) {
        if !config.grants.contains(&Permission::PlaybackControl) {
            self.commands.clear();
        }
        if !config.grants.contains(&Permission::TracksWrite) {
            self.track_overrides.clear();
        }
        if !config.grants.contains(&Permission::UiWrite) {
            self.ui.clear();
            self.ui_slots.clear();
            self.ui_replace.clear();
        }
        if !config.grants.contains(&Permission::BehaviorWrite) {
            self.action_rules.clear();
            self.block_actions.clear();
        }
        if !config.grants.contains(&Permission::NetworkProxy) {
            self.proxy = None;
        }
    }
}

fn engine() -> Engine {
    let mut engine = Engine::new();
    engine.set_max_operations(100_000);
    engine.set_max_call_levels(32);
    engine.set_max_expr_depths(64, 32);
    engine.set_max_string_size(16 * 1024);
    engine.set_max_array_size(4096);
    engine.set_max_map_size(4096);
    engine.set_max_variables(128);
    engine.set_max_functions(128);
    engine.disable_symbol("eval");
    engine.on_print(|_| {});
    engine.on_debug(|_, _, _| {});
    engine
}

// Bound the dynamic tree before serde expands nested/shared script containers.
fn bounded_dynamic(value: &Dynamic, depth: usize, budget: &mut usize, bytes: &mut usize) -> bool {
    if depth > 32 || *budget == 0 {
        return false;
    }
    *budget -= 1;
    if let Some(array) = value.read_lock::<rhai::Array>() {
        return array
            .iter()
            .all(|item| bounded_dynamic(item, depth + 1, budget, bytes));
    }
    if let Some(map) = value.read_lock::<rhai::Map>() {
        return map.iter().all(|(key, item)| {
            let Some(remaining) = bytes.checked_sub(key.len()) else {
                return false;
            };
            *bytes = remaining;
            bounded_dynamic(item, depth + 1, budget, bytes)
        });
    }
    let length = value
        .read_lock::<rhai::ImmutableString>()
        .map_or(8, |text| text.len());
    let Some(remaining) = bytes.checked_sub(length) else {
        return false;
    };
    *bytes = remaining;
    true
}

/// A new scope per invocation prevents scripts accumulating unbounded hidden state.
pub fn evaluate(
    package: &Package,
    config: &PluginConfig,
    event: Value,
    snapshot: Value,
    state: Value,
) -> Result<Output, String> {
    if !config.enabled || config.digest != package.digest {
        return Err("Plugin is disabled".into());
    }
    if !config.grants.is_subset(&package.manifest.permissions) {
        return Err("Undeclared plugin permission".into());
    }
    let snapshot = if config.grants.contains(&Permission::PlaybackRead) {
        snapshot
    } else {
        Value::Null
    };
    let input = serde_json::json!({"event":event,"snapshot":snapshot,"settings":config.settings,"state":state});
    if serde_json::to_vec(&input)
        .map_err(|_| "Invalid plugin input")?
        .len()
        > MAX_PACKAGE
    {
        return Err("Plugin input is too large".into());
    }
    let engine = engine();
    let ast = engine
        .compile(package.source.as_ref())
        .map_err(|_| "Invalid plugin script")?;
    let argument = rhai::serde::to_dynamic(input).map_err(|_| "Invalid plugin input")?;
    let result: Dynamic = engine
        .call_fn(&mut Scope::new(), &ast, "on_event", (argument,))
        .map_err(|_| "Plugin execution failed or exceeded its limits")?;
    let mut bytes = MAX_OUTPUT;
    if !bounded_dynamic(&result, 0, &mut 8192, &mut bytes) {
        return Err("Plugin output is too large or deeply nested".into());
    }
    let mut output: Output =
        rhai::serde::from_dynamic(&result).map_err(|_| "Invalid plugin output")?;
    output.validate()?;
    output.restrict(config);
    Ok(output)
}

#[derive(Clone, Debug)]
pub struct RuntimeResult {
    pub id: String,
    pub digest: String,
    pub revision: u64,
    pub result: Result<Output, String>,
}

enum Request {
    Install {
        serial: u64,
        path: PathBuf,
    },
    Configure {
        serial: u64,
        id: String,
        config: PluginConfig,
    },
    Remove {
        serial: u64,
        id: String,
    },
    Event {
        id: String,
        config: PluginConfig,
        event: Value,
        snapshot: Value,
    },
}

enum Reply {
    Records {
        serial: u64,
        records: Vec<PluginRecord>,
        error: Option<String>,
    },
    Runtime(Box<RuntimeResult>),
}

/// The UI sends bounded jobs; package reads, persistence and scripts stay off-thread.
pub struct PluginManager {
    records: Vec<PluginRecord>,
    requests: SyncSender<Request>,
    replies: Receiver<Reply>,
    serial: u64,
    wake: Arc<dyn Fn() + Send + Sync>,
    pub error: Option<String>,
}

impl PluginManager {
    pub fn new(directory: PathBuf) -> Self {
        Self::with_waker(directory, || {})
    }

    pub fn with_waker(directory: PathBuf, wake: impl Fn() + Send + Sync + 'static) -> Self {
        let (requests, incoming) = mpsc::sync_channel(64);
        let (outgoing, replies) = mpsc::sync_channel(64);
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(wake);
        let outgoing = WakeSender {
            sender: outgoing,
            wake: Arc::clone(&wake),
        };
        std::thread::Builder::new()
            .name("plugins".into())
            .spawn(move || worker(directory, incoming, outgoing))
            .expect("start plugin worker");
        Self {
            records: Vec::new(),
            requests,
            replies,
            serial: 0,
            wake,
            error: None,
        }
    }

    pub fn records(&self) -> &[PluginRecord] {
        &self.records
    }

    pub fn poll(&mut self) -> Vec<RuntimeResult> {
        let mut results = Vec::new();
        let mut received = 0;
        while received < 8
            && let Ok(reply) = self.replies.try_recv()
        {
            received += 1;
            match reply {
                Reply::Records {
                    serial,
                    records,
                    error,
                } if serial == self.serial => {
                    self.records = records;
                    self.error = error;
                }
                Reply::Records { .. } => {}
                Reply::Runtime(result) => {
                    if let Some(record) = self.records.iter_mut().find(|record| {
                        record.package.manifest.id == result.id
                            && record.package.digest == result.digest
                            && record.config.revision == result.revision
                    }) {
                        record.error = result.result.as_ref().err().cloned();
                        results.push(*result);
                    }
                }
            }
        }
        if received == 8 {
            (self.wake)();
        }
        results
    }

    fn send(&mut self, request: Request) -> bool {
        match self.requests.try_send(request) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) => {
                self.error = Some("Plugin worker is busy".into());
                false
            }
            Err(TrySendError::Disconnected(_)) => {
                self.error = Some("Plugin worker stopped".into());
                false
            }
        }
    }

    pub fn install(&mut self, path: PathBuf) {
        let serial = self.serial + 1;
        if self.send(Request::Install { serial, path }) {
            self.serial = serial;
        }
    }

    fn configure(&mut self, id: &str, change: impl FnOnce(&mut PluginRecord)) {
        let Some(index) = self
            .records
            .iter()
            .position(|record| record.package.manifest.id == id)
        else {
            return;
        };
        let mut record = self.records[index].clone();
        change(&mut record);
        let serial = self.serial + 1;
        record.config.revision = serial;
        if self.send(Request::Configure {
            serial,
            id: id.into(),
            config: record.config.clone(),
        }) {
            self.serial = serial;
            record.error = None;
            self.records[index] = record;
        }
    }

    pub fn set_enabled(&mut self, id: &str, enabled: bool) {
        self.configure(id, |record| record.config.enabled = enabled);
    }

    pub fn set_permission(&mut self, id: &str, permission: Permission, allowed: bool) {
        self.configure(id, |record| {
            if allowed && record.package.manifest.permissions.contains(&permission) {
                record.config.grants.insert(permission);
            } else {
                record.config.grants.remove(&permission);
            }
        });
    }

    pub fn set_setting(&mut self, id: &str, key: &str, value: Value) {
        self.configure(id, |record| {
            if record
                .package
                .manifest
                .settings
                .get(key)
                .is_some_and(|setting| setting.accepts(&value))
            {
                record.config.settings.insert(key.into(), value);
            }
        });
    }

    pub fn remove(&mut self, id: &str) {
        let serial = self.serial + 1;
        if self.send(Request::Remove {
            serial,
            id: id.into(),
        }) {
            self.serial = serial;
            self.records
                .retain(|record| record.package.manifest.id != id);
        }
    }

    pub fn permits(&self, id: &str, digest: &str, revision: u64, permission: Permission) -> bool {
        self.records.iter().any(|record| {
            record.package.manifest.id == id
                && record.package.digest == digest
                && record.config.revision == revision
                && record.permits(permission)
        })
    }

    pub fn invoke(&mut self, id: &str, event: Value, snapshot: Value) {
        let Some(record) = self
            .records
            .iter()
            .find(|record| record.package.manifest.id == id && record.config.enabled)
        else {
            return;
        };
        self.send(Request::Event {
            id: id.into(),
            config: record.config.clone(),
            event,
            snapshot,
        });
    }

    pub fn dispatch(&mut self, event: Value, snapshot: Value) {
        let ids: Vec<_> = self
            .records
            .iter()
            .filter(|record| record.config.enabled)
            .map(|record| record.package.manifest.id.clone())
            .collect();
        for id in ids {
            self.invoke(&id, event.clone(), snapshot.clone());
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Stored {
    plugins: BTreeMap<String, PluginConfig>,
}

fn save(directory: &Path, records: &[PluginRecord]) -> Result<(), String> {
    let stored = Stored {
        plugins: records
            .iter()
            .map(|record| (record.package.manifest.id.clone(), record.config.clone()))
            .collect(),
    };
    let bytes =
        serde_json::to_vec_pretty(&stored).map_err(|_| "Unable to encode plugin configuration")?;
    if bytes.len() > MAX_CONFIG {
        return Err("Plugin configuration is too large".into());
    }
    fastframe_file::write(&directory.join("plugins.json"), bytes)
        .map_err(|_| "Unable to save plugin configuration".into())
}

fn remove_obsolete_archives(
    directory: &Path,
    previous: &[PluginRecord],
    current: &[PluginRecord],
) -> Result<(), String> {
    for record in previous {
        if !current
            .iter()
            .any(|kept| kept.package.digest == record.package.digest)
        {
            let path = directory.join(format!("{}.plugin", record.package.digest));
            match std::fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err("Unable to remove plugin archive".into()),
            }
        }
    }
    Ok(())
}

fn load(directory: &Path) -> (Vec<PluginRecord>, Option<String>) {
    let path = directory.join("plugins.json");
    let stored = match read_bounded(&path, MAX_CONFIG) {
        Ok(bytes) => match serde_json::from_slice::<Stored>(&bytes) {
            Ok(stored) => stored,
            Err(_) => return (Vec::new(), Some("Invalid plugin configuration".into())),
        },
        Err(_) if !path.exists() => return (Vec::new(), None),
        Err(error) => return (Vec::new(), Some(error)),
    };
    if stored.plugins.len() > MAX_PLUGINS {
        return (Vec::new(), Some("Too many installed plugins".into()));
    }
    let mut records = Vec::new();
    let mut error = None;
    for (id, mut config) in stored.plugins {
        if !valid_id(&id)
            || config.digest.len() != 64
            || !config.digest.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            error = Some("Invalid plugin configuration".into());
            continue;
        }
        let result = read_bounded(
            &directory.join(format!("{}.plugin", config.digest)),
            MAX_PACKAGE,
        )
        .and_then(Package::from_bytes);
        match result {
            Ok(package) if package.manifest.id == id && package.digest == config.digest => {
                config
                    .grants
                    .retain(|permission| package.manifest.permissions.contains(permission));
                config.settings.retain(|key, value| {
                    package
                        .manifest
                        .settings
                        .get(key)
                        .is_some_and(|setting| setting.accepts(value))
                });
                for (key, setting) in &package.manifest.settings {
                    config
                        .settings
                        .entry(key.clone())
                        .or_insert_with(|| setting.default.clone());
                }
                config.revision = 0;
                records.push(PluginRecord {
                    package,
                    config,
                    error: None,
                });
            }
            _ => error = Some("Unable to load installed plugin".into()),
        }
    }
    (records, error)
}

fn read_bounded(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    let file = std::fs::File::open(path).map_err(|_| "Unable to open plugin file")?;
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Unable to read plugin file")?;
    if bytes.len() > limit {
        return Err("Plugin file is too large".into());
    }
    Ok(bytes)
}

struct WakeSender {
    sender: SyncSender<Reply>,
    wake: Arc<dyn Fn() + Send + Sync>,
}

impl WakeSender {
    fn send(&self, reply: Reply) -> Result<(), mpsc::SendError<Reply>> {
        let result = self.sender.send(reply);
        (self.wake)();
        result
    }
}

fn worker(directory: PathBuf, requests: Receiver<Request>, replies: WakeSender) {
    let (mut records, error) = load(&directory);
    if replies
        .send(Reply::Records {
            serial: 0,
            records: records.clone(),
            error,
        })
        .is_err()
    {
        return;
    }
    let mut states = BTreeMap::<String, Value>::new();
    while let Ok(request) = requests.recv() {
        let (serial, result) = match request {
            Request::Event {
                id,
                config,
                event,
                snapshot,
            } => {
                if let Some(record) = records.iter().find(|record| {
                    record.package.manifest.id == id
                        && record.config.revision == config.revision
                        && record.config.digest == config.digest
                        && record.config.enabled
                }) {
                    let result = evaluate(
                        &record.package,
                        &config,
                        event,
                        snapshot,
                        states.get(&id).cloned().unwrap_or(Value::Null),
                    );
                    if let Ok(output) = &result {
                        states.insert(id.clone(), output.state.clone());
                    }
                    if replies
                        .send(Reply::Runtime(Box::new(RuntimeResult {
                            id,
                            digest: config.digest,
                            revision: config.revision,
                            result,
                        })))
                        .is_err()
                    {
                        return;
                    }
                }
                continue;
            }
            Request::Install { serial, path } => {
                let result = (|| {
                    if path
                        .extension()
                        .is_none_or(|extension| extension != "plugin")
                    {
                        return Err("Choose a .plugin package".into());
                    }
                    let package = Package::from_bytes(read_bounded(&path, MAX_PACKAGE)?)?;
                    let id = package.manifest.id.clone();
                    if records.len() >= MAX_PLUGINS
                        && !records
                            .iter()
                            .any(|record| record.package.manifest.id == id)
                    {
                        return Err("Too many installed plugins".into());
                    }
                    std::fs::create_dir_all(&directory)
                        .map_err(|_| "Unable to create plugin directory")?;
                    fastframe_file::write(
                        &directory.join(format!("{}.plugin", package.digest)),
                        &package.archive,
                    )
                    .map_err(|_| "Unable to install plugin")?;
                    let mut config = PluginConfig {
                        digest: package.digest.clone(),
                        revision: serial,
                        ..Default::default()
                    };
                    for (key, setting) in &package.manifest.settings {
                        config.settings.insert(key.clone(), setting.default.clone());
                    }
                    if let Some(old) = records.iter().find(|record| {
                        record.package.manifest.id == id && record.package.digest == package.digest
                    }) {
                        config = old.config.clone();
                        config.revision = serial;
                    }
                    let mut next = records.clone();
                    next.retain(|record| record.package.manifest.id != id);
                    next.push(PluginRecord {
                        package,
                        config,
                        error: None,
                    });
                    next.sort_by(|a, b| a.package.manifest.id.cmp(&b.package.manifest.id));
                    save(&directory, &next)?;
                    let previous = std::mem::replace(&mut records, next);
                    states.remove(&id);
                    remove_obsolete_archives(&directory, &previous, &records)
                })();
                (serial, result)
            }
            Request::Configure { serial, id, config } => {
                let mut next = records.clone();
                if let Some(record) = next.iter_mut().find(|record| {
                    record.package.manifest.id == id && record.package.digest == config.digest
                }) {
                    record.config = config;
                }
                let result = save(&directory, &next);
                // Revocation applies in memory even when the disk cannot be written.
                records = next;
                states.remove(&id);
                (serial, result)
            }
            Request::Remove { serial, id } => {
                let mut next = records.clone();
                next.retain(|record| record.package.manifest.id != id);
                let result = save(&directory, &next);
                if result.is_ok() {
                    let previous = std::mem::replace(&mut records, next);
                    states.remove(&id);
                    let result = remove_obsolete_archives(&directory, &previous, &records);
                    (serial, result)
                } else {
                    (serial, result)
                }
            }
        };
        if replies
            .send(Reply::Records {
                serial,
                records: records.clone(),
                error: result.err(),
            })
            .is_err()
        {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn archive(manifest: Value, source: &str, extra: Option<&str>) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        writer.start_file("manifest.json", options).unwrap();
        writer
            .write_all(&serde_json::to_vec(&manifest).unwrap())
            .unwrap();
        writer.start_file("main.rhai", options).unwrap();
        writer.write_all(source.as_bytes()).unwrap();
        if let Some(name) = extra {
            writer.start_file(name, options).unwrap();
            writer.write_all(b"bad").unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    fn package(source: &str, permissions: &[&str]) -> Package {
        Package::from_bytes(archive(serde_json::json!({"id":"example","name":"Example","version":"1","api":1,"permissions":permissions}), source, None)).unwrap()
    }

    fn config(package: &Package) -> PluginConfig {
        PluginConfig {
            digest: package.digest.clone(),
            enabled: true,
            grants: package.manifest.permissions.clone(),
            ..Default::default()
        }
    }

    #[test]
    fn package_rejects_path_traversal_and_unknown_permissions() {
        let manifest = serde_json::json!({"id":"example","name":"Example","version":"1","api":1});
        for name in ["../evil", "C:/evil", "foo\\evil", "/evil", "foo/../evil"] {
            assert!(
                Package::from_bytes(archive(
                    manifest.clone(),
                    "fn on_event(input) { #{} }",
                    Some(name)
                ))
                .is_err()
            );
        }
        let bad = serde_json::json!({"id":"example","name":"Example","version":"1","api":1,"permissions":["shell"]});
        assert!(Package::from_bytes(archive(bad, "", None)).is_err());
    }

    #[test]
    fn denied_capabilities_cannot_read_or_write() {
        let package = package(
            "fn on_event(input) { #{state: input.snapshot, commands: [#{type: \"next\"}], proxy: #{mode: \"off\"}} }",
            &["playback_read", "playback_control", "network_proxy"],
        );
        let mut config = config(&package);
        config.grants.clear();
        let output = evaluate(
            &package,
            &config,
            Value::Null,
            serde_json::json!({"track":"private"}),
            Value::Null,
        )
        .unwrap();
        assert!(output.state.is_null());
        assert!(output.commands.is_empty());
        assert!(output.proxy.is_none());
        config.digest = "different".into();
        assert!(evaluate(&package, &config, Value::Null, Value::Null, Value::Null).is_err());
    }

    #[test]
    fn sandbox_stops_loops_recursion_growth_and_eval() {
        for source in [
            "fn on_event(input) { loop {} }",
            "fn on_event(input) { on_event(input) }",
            "fn on_event(input) { let x = \"x\"; loop { x += x; } }",
        ] {
            let package = package(source, &[]);
            assert!(
                evaluate(
                    &package,
                    &config(&package),
                    Value::Null,
                    Value::Null,
                    Value::Null
                )
                .is_err()
            );
        }
        for source in [
            "fn on_event(input) { eval(\"1+1\") }",
            "import \"secret\" as secret; fn on_event(input) { #{} }",
        ] {
            assert!(
                Package::from_bytes(archive(
                    serde_json::json!({"id":"example","name":"Example","version":"1","api":1}),
                    source,
                    None
                ))
                .is_err()
            );
        }
    }

    #[test]
    fn bounded_output_and_real_commands() {
        let package = package(
            "fn on_event(input) { #{commands: [#{type: \"volume\", value: 50}], state: input.settings} }",
            &["playback_control"],
        );
        let output = evaluate(
            &package,
            &config(&package),
            Value::Null,
            Value::Null,
            Value::Null,
        )
        .unwrap();
        assert!(matches!(
            output.commands.as_slice(),
            [PluginCommand::Volume { value: 50 }]
        ));
        assert!(
            !PluginCommand::Open {
                uri: "https://example.com".into()
            }
            .validate()
        );
        assert!(!PluginCommand::Volume { value: 255 }.validate());
    }

    #[test]
    fn localized_text_falls_back_to_language_then_english() {
        let text = LocalizedText::Translations(BTreeMap::from([
            ("en".into(), "Name".into()),
            ("ru".into(), "Имя".into()),
        ]));
        assert_eq!(text.resolve("ru-RU"), "Имя");
        assert_eq!(text.resolve("ar"), "Name");
    }

    #[test]
    fn archive_rejects_duplicates_symlinks_and_expansion() {
        let manifest = serde_json::json!({"id":"example","name":"Example","version":"1","api":1});
        let mut duplicate = archive(manifest.clone(), "", Some("same.rhai"));
        for offset in 0..duplicate.len().saturating_sub(8) {
            if &duplicate[offset..offset + 9] == b"same.rhai" {
                duplicate[offset..offset + 9].copy_from_slice(b"main.rhai");
            }
        }
        assert!(Package::from_bytes(duplicate).is_err());
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        writer
            .add_symlink("link", "target", zip::write::SimpleFileOptions::default())
            .unwrap();
        assert!(Package::from_bytes(writer.finish().unwrap().into_inner()).is_err());
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        writer
            .start_file(
                "bomb",
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated),
            )
            .unwrap();
        writer.write_all(&vec![0; MAX_PACKAGE + 1]).unwrap();
        assert!(Package::from_bytes(writer.finish().unwrap().into_inner()).is_err());
        assert!(Package::from_bytes(archive(manifest, &"x".repeat(MAX_SOURCE + 1), None)).is_err());
    }

    #[test]
    fn dynamic_tree_is_bounded_before_deserialization() {
        for source in [
            "fn on_event(input) { let x = [1]; for i in 0..14 { x = [x, x]; } #{state: x} }",
            "fn on_event(input) { let x = 1; for i in 0..40 { x = [x]; } #{state: x} }",
            "fn on_event(input) { read_file(\"secret\") }",
        ] {
            let package = package(source, &[]);
            assert!(
                evaluate(
                    &package,
                    &config(&package),
                    Value::Null,
                    Value::Null,
                    Value::Null
                )
                .is_err()
            );
        }
    }

    #[test]
    fn settings_and_sdk_example_execute_with_real_snapshot() {
        let manifest: Value = serde_json::from_str(include_str!(
            "../plugins/examples/track-tools/manifest.json"
        ))
        .unwrap();
        let package = Package::from_bytes(archive(
            manifest,
            include_str!("../plugins/examples/track-tools/main.rhai"),
            None,
        ))
        .unwrap();
        let mut config = config(&package);
        config.settings.insert("rename".into(), Value::Bool(true));
        config
            .settings
            .insert("title".into(), Value::String("Renamed".into()));
        let output = evaluate(
            &package,
            &config,
            serde_json::json!({"type":"tick"}),
            serde_json::json!({"playback":{"uri":"spotify:track:abc"}}),
            Value::Null,
        )
        .unwrap();
        assert_eq!(
            output.track_overrides["spotify:track:abc"].name.as_deref(),
            Some("Renamed")
        );
        assert!(matches!(
            &output.ui[0],
            UiNode::Button {
                command: PluginCommand::Next,
                ..
            }
        ));
        config.grants.clear();
        let output = evaluate(&package, &config, Value::Null, Value::Null, Value::Null).unwrap();
        assert!(output.track_overrides.is_empty() && output.ui.is_empty());
        assert!(package.manifest.settings["title"].accepts(&Value::String("ok".into())));
        assert!(!package.manifest.settings["title"].accepts(&Value::Bool(true)));
    }

    #[test]
    fn stale_results_and_full_job_queue_do_not_change_grants() {
        let package = package("fn on_event(input) { #{} }", &["playback_control"]);
        let record = PluginRecord {
            config: config(&package),
            package,
            error: None,
        };
        let (requests, _incoming) = mpsc::sync_channel(1);
        let (outgoing, replies) = mpsc::sync_channel(4);
        let mut manager = PluginManager {
            records: vec![record.clone()],
            requests,
            replies,
            serial: 0,
            error: None,
            wake: Arc::new(|| {}),
        };
        manager.set_permission("example", Permission::PlaybackControl, false);
        assert!(!manager.permits(
            "example",
            &record.package.digest,
            0,
            Permission::PlaybackControl
        ));
        // The full queue rejects a regrant without changing the optimistic config.
        manager.set_permission("example", Permission::PlaybackControl, true);
        assert!(!manager.records[0].permits(Permission::PlaybackControl));
        outgoing
            .send(Reply::Runtime(Box::new(RuntimeResult {
                id: "example".into(),
                digest: record.package.digest,
                revision: 0,
                result: Ok(Output::default()),
            })))
            .unwrap();
        assert!(manager.poll().is_empty());
    }

    #[test]
    fn worker_persists_grants_and_requires_consent_for_changed_package() {
        let directory = std::env::temp_dir().join(format!(
            "spotifast-plugin-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("install.plugin");
        let package = package("fn on_event(input) { #{} }", &["playback_control"]);
        std::fs::write(&path, &package.archive).unwrap();
        let (requests, incoming) = mpsc::sync_channel(8);
        let (outgoing, replies) = mpsc::sync_channel(8);
        let worker_directory = directory.clone();
        let thread = std::thread::spawn(move || {
            worker(
                worker_directory,
                incoming,
                WakeSender {
                    sender: outgoing,
                    wake: Arc::new(|| {}),
                },
            )
        });
        let next = || match replies
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap()
        {
            Reply::Records { records, error, .. } => {
                assert!(error.is_none(), "{error:?}");
                records
            }
            _ => panic!("expected records"),
        };
        assert!(next().is_empty());
        requests
            .send(Request::Install {
                serial: 1,
                path: path.clone(),
            })
            .unwrap();
        let records = next();
        assert!(!records[0].config.enabled && records[0].config.grants.is_empty());
        let mut config = config(&package);
        config.revision = 2;
        requests
            .send(Request::Configure {
                serial: 2,
                id: "example".into(),
                config,
            })
            .unwrap();
        assert!(next()[0].permits(Permission::PlaybackControl));
        let (loaded, error) = load(&directory);
        assert!(error.is_none() && loaded[0].permits(Permission::PlaybackControl));
        requests
            .send(Request::Install {
                serial: 3,
                path: path.clone(),
            })
            .unwrap();
        assert!(next()[0].permits(Permission::PlaybackControl));
        let changed = self::package("fn on_event(input) { #{state: 1} }", &["playback_control"]);
        std::fs::write(&path, &changed.archive).unwrap();
        requests.send(Request::Install { serial: 4, path }).unwrap();
        let records = next();
        assert!(!records[0].config.enabled && records[0].config.grants.is_empty());
        assert!(
            !directory
                .join(format!("{}.plugin", package.digest))
                .exists()
        );
        assert!(
            directory
                .join(format!("{}.plugin", changed.digest))
                .exists()
        );
        requests
            .send(Request::Remove {
                serial: 5,
                id: "example".into(),
            })
            .unwrap();
        assert!(next().is_empty());
        assert!(
            !directory
                .join(format!("{}.plugin", changed.digest))
                .exists()
        );
        drop(requests);
        thread.join().unwrap();
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn proxy_output_rejects_invalid_endpoints() {
        for (mode, host, port, valid) in [
            ("http", "127.0.0.1", "8080", true),
            ("socks", "::1", "1080", true),
            ("off", "", "", true),
            ("system", "", "", true),
            ("http", "", "8080", false),
            ("http", "127.0.0.1", "", false),
            ("http", "127.0.0.1", "0", false),
            ("socks", "127.0.0.1", "65536", false),
            ("http", "user:secret@proxy.example", "80", false),
        ] {
            let output = Output {
                proxy: Some(ProxyPatch {
                    mode: mode.into(),
                    host: host.into(),
                    port: port.into(),
                }),
                ..Default::default()
            };
            assert_eq!(output.validate().is_ok(), valid, "{mode}:{host}:{port}");
        }
    }

    #[test]
    fn configuration_larger_than_package_limit_survives_restart() {
        let directory = std::env::temp_dir().join(format!(
            "spotifast-large-plugin-config-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let mut records = Vec::new();
        for index in 0..12 {
            let settings: BTreeMap<String, Value> = (0..32)
                .map(|index| {
                    (
                        format!("field-{index}"),
                        serde_json::json!({"label":"Text", "kind":"text", "default":""}),
                    )
                })
                .collect();
            let manifest = serde_json::json!({
                "id":format!("example-{index}"), "name":"Example", "version":"1", "api":1,
                "settings":settings
            });
            let package =
                Package::from_bytes(archive(manifest, "fn on_event(input) { #{} }", None)).unwrap();
            std::fs::write(
                directory.join(format!("{}.plugin", package.digest)),
                &package.archive,
            )
            .unwrap();
            let mut config = config(&package);
            config.settings = (0..32)
                .map(|index| (format!("field-{index}"), Value::String("x".repeat(4096))))
                .collect();
            records.push(PluginRecord {
                package,
                config,
                error: None,
            });
        }
        save(&directory, &records).unwrap();
        assert!(
            std::fs::metadata(directory.join("plugins.json"))
                .unwrap()
                .len()
                > MAX_PACKAGE as u64
        );
        let (loaded, error) = load(&directory);
        assert!(error.is_none(), "{error:?}");
        assert_eq!(loaded.len(), records.len());
        assert_eq!(loaded[0].config.settings["field-0"], "x".repeat(4096));
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn polling_limits_frame_work_and_wakes_for_remaining_replies() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let (requests, _) = mpsc::sync_channel(1);
        let (sender, replies) = mpsc::sync_channel(16);
        let wakes = Arc::new(AtomicUsize::new(0));
        let seen = Arc::clone(&wakes);
        let mut manager = PluginManager {
            records: Vec::new(),
            requests,
            replies,
            serial: 0,
            error: None,
            wake: Arc::new(move || {
                seen.fetch_add(1, Ordering::SeqCst);
            }),
        };
        for index in 0..9 {
            sender
                .send(Reply::Records {
                    serial: 0,
                    records: Vec::new(),
                    error: Some(index.to_string()),
                })
                .unwrap();
        }
        assert!(manager.poll().is_empty());
        assert_eq!(manager.error.as_deref(), Some("7"));
        assert_eq!(wakes.load(Ordering::SeqCst), 1);
        manager.poll();
        assert_eq!(manager.error.as_deref(), Some("8"));
    }

    #[test]
    fn command_batches_share_the_output_effect_budget() {
        let batch = PluginCommand::QueueMany {
            uris: vec!["spotify:track:abc".into(); 32],
        };
        let mut output = Output {
            commands: vec![batch.clone()],
            ..Default::default()
        };
        assert!(output.validate().is_ok());
        output.commands.push(PluginCommand::Next);
        assert!(output.validate().is_err());
        assert!(
            !PluginCommand::QueueMany {
                uris: vec!["spotify:track:abc".into(); 33]
            }
            .validate()
        );
        for uri in [
            "spotify:album:abc",
            "spotify:artist:abc",
            "spotify:playlist:abc",
            "spotify:show:abc",
        ] {
            assert!(
                !PluginCommand::QueueMany {
                    uris: vec![uri.into()]
                }
                .validate()
            );
            assert!(
                !PluginCommand::PlayNext {
                    uris: vec![uri.into()]
                }
                .validate()
            );
        }
    }
}
