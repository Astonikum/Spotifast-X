//! Apply sandboxed extension effects through the same actions as native controls.
use super::*;
use crate::plugins::{Permission, PluginCommand, PluginManager, RuntimeResult};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub struct PluginHost {
    pub manager: PluginManager,
    pub outputs: BTreeMap<String, RuntimeResult>,
    last_dispatch: Instant,
    pub applying: bool,
    proxy: Option<crate::settings::ProxyConfig>,
}

impl PluginHost {
    pub fn new(directory: PathBuf, waker: Waker) -> Self {
        Self {
            manager: PluginManager::with_waker(directory, move || waker.wake()),
            outputs: BTreeMap::new(),
            last_dispatch: Instant::now() - Duration::from_secs(1),
            applying: false,
            proxy: None,
        }
    }

    pub fn permits(&self, result: &RuntimeResult, permission: Permission) -> bool {
        self.manager
            .permits(&result.id, &result.digest, result.revision, permission)
    }
}

impl App {
    #[cfg(test)]
    pub(super) fn defer_plugin_ticks(&mut self, duration: Duration) {
        self.plugins.last_dispatch = Instant::now() + duration;
    }

    pub(crate) fn invalidate_plugin(&mut self, id: &str) {
        self.plugins.outputs.remove(id);
        self.plugins.last_dispatch = Instant::now() - Duration::from_secs(1);
        self.sync_plugin_proxy();
    }

    pub(super) fn plugin_snapshot(&self) -> Value {
        let playback = self.now_playing().map(|now| {
            json!({
                "uri": now.uri, "title": now.title,
                "artists": now.artists.iter().map(|artist| &artist.name).collect::<Vec<_>>(),
                "album": now.album_name, "duration_ms": now.duration_ms,
                "position_ms": now.position_ms, "playing": now.playing,
                "shuffle": now.shuffle, "repeat": now.repeat.api_name(),
                "volume": now.volume_percent,
            })
        });
        let mut tracks: Vec<_> = self.track_cache.values().collect();
        tracks.sort_by(|a, b| a.uri.cmp(&b.uri));
        let tracks: Vec<_> = tracks.into_iter().take(256).map(|track| json!({
            "uri": track.uri, "name": track.name, "artists": track.artists.iter().map(|a| &a.name).collect::<Vec<_>>(),
            "album": track.album.as_ref().map(|album| &album.name), "duration_ms": track.duration_ms,
        })).collect();
        let queue: Vec<_> =
            self.queue
                .get()
                .map(|queue| {
                    queue.queue.iter().take(256).map(|item| json!({
            "uri":item.uri(), "name":item.name(), "duration_ms":item.duration_ms(),
        })).collect()
                })
                .unwrap_or_default();
        json!({ "playback": playback, "tracks": tracks, "queue": queue, "page": self.page().encode(),
            "locale": self.locale.tag(), "theme": if self.palette.dark { "dark" } else { "light" } })
    }

    pub(crate) fn tick_plugins(&mut self, ctx: &egui::Context) {
        for result in self.plugins.manager.poll() {
            if let Ok(output) = &result.result {
                let commands = if self.plugins.permits(&result, Permission::PlaybackControl) {
                    output.commands.clone()
                } else {
                    Vec::new()
                };
                // Preserve each result's generation; revocation also invalidates cached UI and policies.
                let id = result.id.clone();
                self.plugins.outputs.insert(id, result);
                for command in commands {
                    self.apply_plugin_command(command, ctx);
                }
            }
        }
        self.sync_plugin_proxy();
        if !self.plugins.manager.records().is_empty()
            && self.plugins.last_dispatch.elapsed() >= Duration::from_secs(1)
        {
            self.plugins.last_dispatch = Instant::now();
            let snapshot = self.plugin_snapshot();
            self.plugins
                .manager
                .dispatch(json!({"type": "tick"}), snapshot);
        }
        // Installation and script evaluation never wait for a frame or touch the UI thread.
        if !self.plugins.manager.records().is_empty() {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }

    pub(crate) fn plugin_action_override(&self, action: &Action) -> Option<PluginCommand> {
        if self.plugins.applying {
            return None;
        }
        let name = plugin_action_name(action)?;
        self.plugins.outputs.values().find_map(|result| {
            if !self.plugins.permits(result, Permission::BehaviorWrite)
                || !self.plugins.permits(result, Permission::PlaybackControl)
            {
                return None;
            }
            result.result.as_ref().ok()?.action_rules.get(name).cloned()
        })
    }

    pub(crate) fn plugin_action_blocked(&self, action: &Action) -> bool {
        if self.plugins.applying {
            return false;
        }
        let Some(name) = plugin_action_name(action) else {
            return false;
        };
        self.plugins.outputs.values().any(|result| {
            self.plugins.permits(result, Permission::BehaviorWrite)
                && result
                    .result
                    .as_ref()
                    .is_ok_and(|output| output.block_actions.contains(name))
        })
    }

    pub(crate) fn emit_plugin_action(&mut self, action: &Action) {
        if self.plugins.applying {
            return;
        }
        if let Some(name) = plugin_action_name(action) {
            let snapshot = self.plugin_snapshot();
            self.plugins
                .manager
                .dispatch(json!({"type":"action", "name":name}), snapshot);
        }
    }

    pub(crate) fn apply_plugin_command(&mut self, command: PluginCommand, ctx: &egui::Context) {
        let action = match command {
            PluginCommand::Play { uri }
                if uri.starts_with("spotify:track:") || uri.starts_with("spotify:episode:") =>
            {
                Action::PlayUris {
                    uris: vec![uri],
                    index: 0,
                }
            }
            PluginCommand::Play { uri } => Action::PlayContext {
                uri,
                offset_uri: None,
                offset_index: None,
            },
            PluginCommand::Pause if self.now_playing().is_some_and(|now| now.playing) => {
                Action::TogglePlay
            }
            PluginCommand::Resume if self.now_playing().is_some_and(|now| !now.playing) => {
                Action::TogglePlay
            }
            PluginCommand::Pause | PluginCommand::Resume => return,
            PluginCommand::Next => Action::Next,
            PluginCommand::Previous => Action::Previous,
            PluginCommand::Seek { position_ms } => Action::Seek(position_ms),
            PluginCommand::Volume { value } => Action::SetVolume(value.min(100)),
            PluginCommand::Shuffle { enabled } => Action::SetShuffle(enabled),
            PluginCommand::Repeat { mode } => Action::SetRepeat(match mode.as_str() {
                "off" => RepeatMode::Off,
                "context" => RepeatMode::Context,
                "track" => RepeatMode::Track,
                _ => return,
            }),
            PluginCommand::Queue { uri } => Action::AddToQueue {
                uri,
                label: String::new(),
            },
            PluginCommand::Open { uri } => Action::OpenUri(uri),
            PluginCommand::QueueMany { uris } => Action::QueueMany {
                songs: uris.into_iter().map(|uri| (uri, String::new())).collect(),
            },
            PluginCommand::PlayNext { uris } => {
                let items = uris
                    .into_iter()
                    .map(|uri| self.optimistic_queue_item(&uri, ""))
                    .collect();
                Action::PlayNext { items }
            }
            PluginCommand::ClearQueue => Action::ClearQueue,
            PluginCommand::OpenPage { page } => match Page::decode(&page) {
                Some(page) => Action::Open(page),
                None => return,
            },
            PluginCommand::Search { query } => Action::Search(query),
            PluginCommand::Emit { .. } => return,
        };
        self.plugins.applying = true;
        self.apply(action, ctx);
        self.plugins.applying = false;
    }

    pub(crate) fn plugin_track_patch(&self, mut now: NowPlaying) -> NowPlaying {
        for result in self.plugins.outputs.values() {
            if !self.plugins.permits(result, Permission::TracksWrite) {
                continue;
            }
            let Some(patch) = result
                .result
                .as_ref()
                .ok()
                .and_then(|output| output.track_overrides.get(&now.uri))
            else {
                continue;
            };
            if let Some(name) = &patch.name {
                now.title.clone_from(name);
            }
            if let Some(album) = &patch.album {
                now.album_name.clone_from(album);
            }
            if let Some(artists) = &patch.artists {
                now.artists = artists
                    .iter()
                    .map(|name| ArtistRef {
                        name: name.clone(),
                        ..ArtistRef::default()
                    })
                    .collect();
                now.subtitle = artists.join(", ");
            }
        }
        now
    }

    pub(crate) fn plugin_item_patch(&self, item: &PlayableItem) -> Option<PlayableItem> {
        let mut changed = None;
        for result in self.plugins.outputs.values() {
            if !self.plugins.permits(result, Permission::TracksWrite) {
                continue;
            }
            let Some(patch) = result
                .result
                .as_ref()
                .ok()
                .and_then(|output| output.track_overrides.get(item.uri()))
            else {
                continue;
            };
            let copy = changed.get_or_insert_with(|| item.clone());
            if let PlayableItem::Track(track) = copy {
                if let Some(name) = &patch.name {
                    track.name.clone_from(name);
                }
                if let Some(artists) = &patch.artists {
                    track.artists = artists
                        .iter()
                        .map(|name| ArtistRef {
                            name: name.clone(),
                            ..ArtistRef::default()
                        })
                        .collect();
                }
                if let Some(name) = &patch.album
                    && let Some(album) = &mut track.album
                {
                    album.name.clone_from(name);
                }
            }
        }
        changed
    }

    fn sync_plugin_proxy(&mut self) {
        let desired = self.plugins.outputs.values().find_map(|result| {
            if !self.plugins.permits(result, Permission::NetworkProxy) {
                return None;
            }
            let proxy = result.result.as_ref().ok()?.proxy.as_ref()?;
            let mode = match proxy.mode.as_str() {
                "off" => crate::settings::ProxyMode::Off,
                "system" => crate::settings::ProxyMode::System,
                "http" => crate::settings::ProxyMode::Http,
                "socks" => crate::settings::ProxyMode::Socks,
                _ => return Some(Err("Invalid plugin proxy mode".into())),
            };
            let settings = Settings {
                proxy_mode: mode,
                proxy_host: proxy.host.clone(),
                proxy_port: proxy.port.clone(),
                ..Settings::default()
            };
            Some(settings.proxy_config())
        });
        let desired = match desired.transpose() {
            Ok(desired) => desired,
            Err(error) => {
                self.plugins.manager.error = Some(error);
                return;
            }
        };
        if desired != self.plugins.proxy {
            let config = desired
                .clone()
                .unwrap_or_else(|| self.applied_proxy.clone());
            self.plugins.proxy = desired;
            self.backend.send(Command::PluginProxy(config));
        }
    }

    pub(super) fn reapply_plugin_proxy(&mut self) {
        if self.plugins.proxy.take().is_some() {
            self.sync_plugin_proxy();
        }
    }
}

fn plugin_action_name(action: &Action) -> Option<&'static str> {
    Some(match action {
        Action::PlayContext { .. }
        | Action::PlayUris { .. }
        | Action::PlayFromRow { .. }
        | Action::PlayEpisode { .. } => "play",
        Action::AddToQueue { .. }
        | Action::QueueMany { .. }
        | Action::PlayNext { .. }
        | Action::ClearQueue => "queue",
        Action::Next => "next",
        Action::Previous => "previous",
        Action::TogglePlay => "toggle_play",
        Action::Seek(_) | Action::SeekBy(_) => "seek",
        Action::SetVolume(_) | Action::VolumeBy(_) => "volume",
        Action::ToggleShuffle | Action::SetShuffle(_) => "shuffle",
        Action::CycleRepeat | Action::SetRepeat(_) => "repeat",
        _ => return None,
    })
}
