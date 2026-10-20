---
title: Plugins
description: Install plugins, choose permissions, and develop a plugin.
nav_order: 7
---

## Install

Open **Settings → Plugins → Install plugin** and choose a `.plugin` file.
Each plugin is a ZIP archive containing a manifest and Rhai code.

Review its requested permissions, grant only what you need, then enable it.
Use the same section to change its settings, revoke permissions, disable it,
or remove it. Permissions are stored separately for each plugin.

Installing a changed package replaces the plugin, resets its settings and
clears its permissions. Review and grant them again before enabling it.
Reinstalling the identical package preserves its configuration.
Removing a plugin deletes its configuration and stored archive. Updating it
removes the previous archive after the new configuration is saved.

Errors appear in the plugin's settings. An execution error keeps the last
successful effects until the next valid result, disabling, or permission change.

## Develop

Plugins use Rhai, an embedded scripting language. They run locally without a
Python, JavaScript, or Rust installation. The host validates their output and
checks permissions before applying changes.

See the [Plugin API](/plugin-api/) for the manifest, hooks, permissions, and
limits. The [example source](https://github.com/Astonikum/Spotifast-X/tree/main/plugins/examples/track-tools)
can be copied and modified.
