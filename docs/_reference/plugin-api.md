---
title: Plugin API
description: Manifest, Rhai hooks, permissions, settings, and host limits.
nav_order: 7
---

API version **1** uses Rhai scripts and JSON-compatible values. The source of
truth is [`src/plugins.rs`](https://github.com/Astonikum/Spotifast-X/blob/main/src/plugins.rs)
and [`src/app_plugins.rs`](https://github.com/Astonikum/Spotifast-X/blob/main/src/app_plugins.rs).

## Package

A `.plugin` file is a ZIP archive with UTF-8 `manifest.json` and `main.rhai`
at its root, without a containing folder. Only `main.rhai` executes;
imports and additional executable modules are unavailable.

Copy the [example](https://github.com/Astonikum/Spotifast-X/tree/main/plugins/examples/track-tools).
From its directory, package it with Python's standard library:

```sh
python -m zipfile -c track-tools.plugin manifest.json main.rhai
python -m zipfile -t track-tools.plugin
```

Python is needed only for this command. ZIP tools work too: zip the two
files, then change `.zip` to `.plugin`. Install through **Settings → Plugins**.
The example adds a **Next** button and optionally renames the displayed track.
Each effect requires its corresponding permission.

## Manifest

```json
{
  "id": "my-plugin",
  "name": { "en": "My plugin", "ru": "Мой плагин" },
  "version": "1.0.0",
  "api": 1,
  "entry": "main.rhai",
  "permissions": ["playback_read", "ui_write"],
  "settings": {
    "enabled": {
      "label": { "en": "Enabled", "ru": "Включено" },
      "kind": "bool",
      "default": true
    }
  }
}
```

| Field | Contract |
| --- | --- |
| `id` | Stable identity, 1–64 lowercase ASCII letters, digits or `-`; starts with a letter. Installing the same ID replaces that plugin. |
| `name` | Localized text. |
| `version` | Nonempty display string, at most 64 bytes. No automatic version ordering. |
| `api` | Must be `1`. |
| `entry` | Optional; only `main.rhai` is accepted. |
| `permissions` | Optional array of permissions below. Requests do not grant access. |
| `settings` | Optional object, at most 32 entries; keys follow the `id` rules. |

Unknown manifest and setting fields are rejected. Localized text is a plain
string or a map of language tags to strings. Translation maps require `en`.
Resolution tries the exact locale, its base language, then English. Each text
must be nonempty and at most 1,024 bytes. Provide translations for your users;
keep labels short, without subtitles or slogans. Native controls inherit the
host theme and text direction.

Setting definitions contain `label`, `kind`, and `default`:

| `kind` | Value | Extra fields |
| --- | --- | --- |
| `bool` | JSON boolean | None |
| `text` | String, at most 4,096 bytes | None |
| `number` | Finite number | Optional inclusive `min` and `max` |
| `select` | One of `options` | `options`: 1–64 JSON values, at most 8 KiB combined |

Defaults and edits use the same validation. Settings appear under the plugin
in **Settings → Plugins** and arrive in `input.settings`. They are ordinary
readable configuration, unsuitable for passwords or tokens.

## Permissions

| Permission | Allows |
| --- | --- |
| `playback_read` | Playback, page, locale and theme snapshot. |
| `playback_control` | Commands and command buttons. |
| `tracks_write` | Track display overrides. |
| `network_proxy` | Temporary proxy override. This can redirect application traffic through a plugin-selected server. |
| `ui_write` | Native UI at supported slots. Buttons executing host commands also require `playback_control`; `emit` buttons do not. |
| `behavior_write` | Replacement control actions; also requires `playback_control`. |

New packages start disabled with no grants. Grants bind to the complete ZIP's
SHA-256 digest, not just its ID or version. Changing any archive bytes requires
consent again and resets plugin settings to defaults. Reinstalling identical
bytes preserves configuration. Plugins cannot grant themselves permissions.
Revoking a grant or disabling a plugin invalidates its queued effects and
cached UI. Permission changes do not undo commands already performed.

## Hook

Define one function:

```rhai
fn on_event(input) {
    #{state: input.state, ui: []}
}
```

The host invokes it off the UI thread approximately once per second while the
app is running. Do not send a playback command every tick unless repetition
is intentional. Events are maps:

| `type` | Additional fields | Trigger |
| --- | --- | --- |
| `tick` | None | Periodic update |
| `action` | `name` | Supported native control action, before applying it |
| `custom` | `name` | This plugin's button with an `emit` command |

| Input | Value |
| --- | --- |
| `event` | Event map. |
| `snapshot` | Map below, or Rhai unit `()` without `playback_read`. |
| `settings` | Map of validated settings, including defaults. |
| `state` | Last successful output's `state`, initially `()`. |

Snapshot shape:

```json
{
  "page": "home",
  "locale": "en",
  "theme": "dark",
  "tracks": [],
  "queue": [],
  "playback": {
    "uri": "spotify:track:TRACK_ID",
    "title": "Title",
    "artists": ["Artist"],
    "album": "Album",
    "duration_ms": 200000,
    "position_ms": 10000,
    "playing": true,
    "shuffle": false,
    "repeat": "off",
    "volume": 50
  }
}
```

`playback` is `()` when nothing is playing. `theme` is `light` or `dark`;
custom colours remain managed by the host. `page` is the host's encoded page
identifier. Treat it as a string rather than relying on undocumented values.
`tracks` contains up to 256 cached tracks, sorted by URI, with `uri`, `name`,
`artists`, `album` and `duration_ms`. `queue` contains up to 256 queued items
with `uri`, `name` and `duration_ms`. An absent album is `()`.

Each invocation starts with a fresh script scope. Return cross-event data in
`state`; it stays in memory and resets on restart, update or configuration
change. A result replaces that plugin's previous declarative output. Omitted
fields use defaults, so omit `proxy`, `ui` or overrides to remove those effects.
Unknown output fields and invalid commands reject the result.

Effects are combined in plugin-ID order. The first permitted proxy or action
replacement wins. Track patches apply in that order, with later fields
overriding earlier fields. Action blocks combine; UI from all permitted
plugins is drawn.

## Output

```rhai
#{
    state: #{count: 1},
    commands: [],
    track_overrides: #{},
    ui: [],
    ui_slots: #{},
    ui_replace: [],
    block_actions: [],
    action_rules: #{}
}
```

### Commands

Commands are maps tagged by `type`:

| `type` | Fields |
| --- | --- |
| `play`, `queue`, `open` | `uri` |
| `queue_many`, `play_next` | `uris`: 1–32 track or episode Spotify URIs |
| `pause`, `resume`, `next`, `previous`, `clear_queue` | None |
| `seek` | `position_ms`: unsigned 32-bit integer |
| `volume` | `value`: integer from 0 to 100 |
| `shuffle` | `enabled`: boolean |
| `repeat` | `mode`: `off`, `track` or `context` |
| `open_page` | `page`: `home`, `top-songs`, `new-releases`, `search`, `liked`, `albums`, `artists`, `podcasts`, `episodes`, `queue` or `settings` |
| `search` | `query`: nonempty string, at most 1,024 bytes |
| `emit` | `event`: nonempty name, at most 128 bytes; UI buttons only |

URIs must be `spotify:<kind>:<id>`; accepted kinds are `track`, `episode`,
`album`, `playlist`, `artist`, and `show`. The command still needs to make
sense for that kind and satisfy the host's playback restrictions. Commands
use normal application actions, including optimistic UI updates. `open`
navigates inside the app. Commands emitted by a plugin bypass action
replacement rules to prevent recursion.
`queue` accepts only track and episode URIs.
`emit` invokes only the emitting plugin, with `input.event` set to
`#{type: "custom", name: "your-event"}`. It is invalid in `commands` or
`action_rules`. Use it for buttons that need script logic rather than a fixed
playback command.

### Track display

```rhai
let patches = #{};
patches["spotify:track:TRACK_ID"] = #{
    name: "Title", artists: ["Artist"], album: "Album"
};
#{track_overrides: patches}
```

All patch fields are optional. Overrides affect display, not Spotify's
catalogue, audio, or saved playlist metadata.
Names and album titles allow 1,024 bytes each; `artists` allows at most 32
names of 1,024 bytes each.

### Native UI

Nodes use `type: "label"` with `text`, `type: "button"` with `text` and
`command`, or `type: "row"` / `"column"` with a `children` array. `text`
accepts localized text. For example:

```rhai
#{ui: [#{
    type: "button",
    text: #{en: "Next", ru: "Далее"},
    command: #{type: "next"}
}]}
```

The host draws widgets with its existing components and theme. There is no
HTML, JavaScript DOM, embedded browser, or raw egui access.

`ui` appends to `page_end`. To choose another location, use `ui_slots`:

```rhai
#{ui_slots: #{player_bar: [#{
    type: "button",
    text: #{en: "Next", ru: "Далее"},
    command: #{type: "next"}
}]}}
```

Supported slots are `page_start`, `page_end`, `sidebar`, `player_bar`,
`topbar`, and `settings`. `ui_replace: ["page"]` hides the native page
content while retaining the application shell and plugin widgets. Settings
always remains accessible and cannot be replaced.

### Control rules

```rhai
#{action_rules: #{next: #{type: "pause"}}}
```

Rules replace a named native action with one command. Supported keys are
`play`, `toggle_play`, `next`, `previous`, `seek`, `volume`, `shuffle`,
`repeat`, and `queue`. A replacement does not execute the original action.
Plugin-issued commands bypass these rules.
`block_actions` is an array of action names to suppress. Supported action
names are `play`, `toggle_play`, `next`, `previous`, `seek`, `volume`,
`shuffle`, `repeat`, and `queue`. Blocks require `behavior_write` and are
applied before replacements. To implement an algorithm, block its native
action, handle the corresponding `action` event, and return the desired
commands. These commands bypass the plugin's blocks.

Hooks run asynchronously. An `action` event alone does not delay or cancel
the native action; the preceding successful output must already supply its
block or replacement rule. Events supply the action name, without its
arguments. Rules cannot intercept arbitrary Rust functions or backend tasks.

### Proxy

```rhai
#{proxy: #{mode: "http", host: "127.0.0.1", port: "8080"}}
```

Modes are `system`, `off`, `http`, and `socks`. Manual modes use a host and
string port, validated by native proxy rules. Plugin proxy overrides are
temporary, contain no username or password, and do not rewrite saved proxy
settings. Disabling the plugin, revoking its permission, or removing its
override restores the applicable saved proxy configuration.
Startup uses saved proxy settings until an enabled plugin supplies its first valid override.
An invalid proxy rejects the result and keeps the last successful route;
there is no silent fallback to a direct connection. These overrides use the
same transport capabilities and local-playback restrictions as
[native proxy settings](/how-it-connects/#proxy).

## Limits

| Resource | Limit |
| --- | --- |
| Installed plugins | 32 |
| Archive and total expanded bytes | 1 MiB each |
| Archive entries | 32; no duplicate names, traversal paths or symlinks |
| Manifest / script | 32 KiB / 128 KiB |
| Output | 128 KiB |
| Commands / track overrides / action rules per output | 32 / 512 / 16; each batch URI counts as one command |
| UI nodes / nesting | 128 / 8 |
| Rhai operations per invocation / call depth | 100,000 / 32 |
| Rhai string size / array size / map size | 16 KiB / 4,096 / 4,096 |
| Input / returned value tree | 1 MiB / 8,192 values, nesting at most 32 |

No arbitrary filesystem, shell, socket, HTTP, credential, native-memory or
Rust-code access is provided. Hooks change only supported host surfaces.
Permissions cannot make unsupported operations available. Plugins cannot
remove Spotify or librespot limits; see [What Spotify allows](/what-spotify-allows/).

Errors appear in **Settings → Plugins**. Fix the source and reinstall the
package. An execution error keeps the last successful output and state;
its commands are not repeated. The next valid result replaces those effects.
Disabling, removing, or changing a plugin's permissions clears cached effects;
completed commands remain. Successful updates and removals delete the old
stored archive after saving the new configuration atomically.
