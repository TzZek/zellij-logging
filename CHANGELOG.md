# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Initial release. Modeled on `tmux-plugins/tmux-logging`.
- Continuous logging via `Event::PaneRenderReport` with viewport-diff append.
- One-shot visible-viewport snapshot (`snapshot` pipe message).
- One-shot full-scrollback dump (`dump_full` pipe message).
- `clear_history` pipe message (opt-in via `enable_clear_history`) that
  wipes the focused pane's scrollback. Requests `ChangeApplicationState`
  only when the feature is enabled.
- Filename templates: `{session}`, `{tab}`, `{pane_id}`, `{pane_title}`,
  `{ts}`, `{date}`, `{time}`.
- Configurable output directory (defaults to `/host/zellij-logs`).
- Per-line ISO-8601 timestamps with millisecond precision and timezone
  offset, computed per line. Toggle via `timestamp_lines` config.
- ANSI escape stripping (toggle via config; default on).
- `auto_start` config option to begin logging the focused pane on plugin load.

### Requires
- Zellij 0.44.2 specifically. The plugin ABI is auto-generated from
  protobuf and changes between point releases; `zellij-tile` is pinned
  to `=0.44.2` in `Cargo.toml`. To upgrade Zellij, bump the pin to match
  and rebuild. Zellij 0.44.0 / 0.44.1 will load the wasm but fail at
  plugin-init with `could not find exported function`.
