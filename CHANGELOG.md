# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.1] - 2026-05-08

Security fix release. v0.1.0 was published roughly an hour earlier and had
no deployed users, so no security advisory; the fix is rolled into this
patch release.

## [Unreleased]

### Added
- Initial release. Modeled on `tmux-plugins/tmux-logging`.
- Continuous logging keyed on `Event::PaneRenderReport` events, but the
  capture itself uses `get_pane_scrollback(pane_id, get_full_scrollback=true)`
  to fetch the complete `lines_above_viewport ++ viewport` history each
  cycle. This catches content that scrolled past the viewport too fast for
  a viewport-only snapshot, which is the behaviour engagement-grade
  logging actually needs. Toggle-on captures the current scrollback as a
  baseline so the log only contains content produced after that moment,
  matching `tmux-logging`'s `pipe-pane` semantics.
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
- `clean_log` config option (default `true`) that writes a sibling
  `<basename>.clean.log` for each continuous log: no per-line timestamps,
  ANSI escapes always stripped. The primary timestamped log is unaffected;
  this is purely a readable companion file.
- `visual_indicator` config option (default `true`) that calls
  `highlight_and_unhighlight_panes` to mark tracked panes when toggled
  on, and unhighlights them when toggled off. Auto-requests
  `ChangeApplicationState` permission only when this option (or
  `enable_clear_history`) is enabled.
- `SECURITY.md` documenting the threat model, disclosure process, and
  the list of permissions the plugin does and does not request.

### Security

- Path traversal via pane titles fixed. A program running in a tracked
  pane can rewrite its own title via OSC escape sequences; previously,
  if the user's `filename_template` referenced `{pane_title}`, a title
  like `../../../etc/passwd` could direct writes outside `output_dir`.
  The template `sanitise()` function now strips `/` and `\` from
  substituted values and collapses any surviving `..` runs.

### Requires
- Zellij 0.44.2 specifically. The plugin ABI is auto-generated from
  protobuf and changes between point releases; `zellij-tile` is pinned
  to `=0.44.2` in `Cargo.toml`. To upgrade Zellij, bump the pin to match
  and rebuild. Zellij 0.44.0 / 0.44.1 will load the wasm but fail at
  plugin-init with `could not find exported function`.
