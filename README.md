<p align="center">
  <img src="docs/assets/images/logo.svg" alt="Spotifast X logo" width="88" height="88">
</p>

<h1 align="center">Spotifast X</h1>

<p align="center">
  <a href="https://github.com/Astonikum/Spotifast-X/releases"><strong>Download</strong></a> ·
  <a href="docs/_guide/getting-started.md">Getting started</a> ·
  <a href="docs/_guide/using-spotifast.md">User guide</a> ·
  <a href="CONTRIBUTING.md">Contribute</a>
</p>

<p align="center">
  <a href="Cargo.toml"><img src="https://img.shields.io/badge/Built_with-Rust-1ed760" alt="Built with Rust"></a>
  <a href="https://github.com/Astonikum/Spotifast-X/releases"><img src="https://img.shields.io/badge/Linux_%C2%B7_macOS_%C2%B7_Windows-24292f" alt="Linux, macOS, and Windows"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-MIT-1ed760" alt="MIT license"></a>
</p>

Spotifast X is a Spotify client written in Rust with
[egui](https://www.egui.rs) and [fastframe](https://fastframe.dev), the
framework behind a family of native desktop apps. It plays music through
[librespot](https://github.com/librespot-org/librespot), typically uses
100–250 MB of RAM, starts in well under a second, and has no browser engine.

The interface includes complete
[translations for all supported languages](docs/_reference/translating.md#languages-and-coverage).

**Playback needs Spotify Premium.** Free accounts can browse and search, but
cannot play music through Spotifast X.

![Spotifast X Home with the playlist library, recommendations, queue, and player visible](docs/screenshot.png)

<details>
<summary><strong>Watch Spotifast X in action</strong></summary>

https://github.com/user-attachments/assets/a5f669ce-b3b7-4f8e-9933-976a78876c7e

</details>

[Features](#features) · [Install](#install) · [Help](#guides-and-help)

## Features

| Feature | What you can do |
|---|---|
| **Library and search** | Browse playlists, Liked Songs, albums, artists, and podcasts. Search the catalogue and edit playlists you own. |
| **Local files** | Play the local files your playlists list from a folder you choose in Settings (44.1 kHz files). |
| **Spotify Connect** | Play on this computer or control playback on your other devices, including Google Cast speakers and groups. Cast devices appear as they are discovered; frequently and recently chosen outputs rise to the top. |
| **Queue** | Add songs to the queue, or choose **Play next** to put a song or selection first when playing on this computer. |
| **Themes** | Choose light, dark, system appearance, or custom colours, with one theme for light mode and one for dark mode that switch with the system. On Omarchy, follow your desktop theme. |
| **Desktop controls** | Use keyboard shortcuts and media keys. Keep music playing from the tray when supported by your desktop and settings, and start Spotifast X with your computer, with its window or in the tray. |

## Install

| Installation | Link |
|---|---|
| **Downloads** | [Releases](https://github.com/Astonikum/Spotifast-X/releases) |
| **From source** | [Build from source](docs/_guide/getting-started.md#build-from-source) |

## Guides and help

Documentation: [docs](docs/).

| You want to… | Read |
|---|---|
| Sign in, set up playback, or configure themes, fonts, and proxies | [Getting started](docs/_guide/getting-started.md) |
| Learn shortcuts, command-line controls, and updates | [Everyday use](docs/_guide/using-spotifast.md) |
| Find configuration and stored files | [Settings and files](docs/_reference/settings-and-files.md) |
| Understand scrolling performance and reduce loading delays | [Make it even faster](docs/_guide/make-it-even-faster.md) |
| Understand what is stored and sent | [Privacy](docs/_reference/privacy.md) · [How it connects](docs/_reference/how-it-connects.md) |
| Check Spotify and librespot limitations | [What Spotify allows](docs/_reference/what-spotify-allows.md) |
| Understand account risk | [Will my account get banned?](docs/_guide/what-is-spotifast.md#will-my-spotify-account-get-banned) |
| Report a bug or propose a feature | Read [Contributing](CONTRIBUTING.md), then use the [issue forms](https://github.com/crmne/spotifast/issues/new/choose). |

### Plugin development

[Guide](docs/_guide/plugins.md) · [API](docs/_reference/plugin-api.md) · [Example](plugins/examples/track-tools/)

## Contributing

Read [CONTRIBUTING.md](CONTRIBUTING.md) before opening an issue or pull
request. To look at the interface without a Spotify account, run:

```sh
cargo run --features demo -- --demo
```

Translations live in `assets/i18n/`; see
[Translating Spotifast X](docs/_reference/translating.md). Release packaging
is described in [PACKAGING.md](PACKAGING.md).

## Sonos playback

Sonos speakers use the Spotify Connect session for playback control. Enable
local playback first, then select the speaker in the device picker.

## Acknowledgements

Spotifast X uses [librespot](https://github.com/librespot-org/librespot),
[egui](https://www.egui.rs), [fastframe](https://fastframe.dev), the [Inter](https://rsms.me/inter/)
typeface (OFL), and [Lucide](https://lucide.dev) icons (ISC).

Spotifast X is an independent project and is not affiliated with Spotify.
Spotify is a trademark of Spotify AB.

Licensed under the [MIT License](LICENSE).
