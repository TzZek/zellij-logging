# zellij-logging

A [Zellij](https://zellij.dev) plugin that logs pane output to disk.
Modeled on [`tmux-plugins/tmux-logging`](https://github.com/tmux-plugins/tmux-logging),
which is the de-facto session-logging plugin in the tmux ecosystem.

Built for pentesters, red teamers, sysadmins, and anyone who wants a paper
trail of what happened in a pane. Continuous logging, one-shot viewport
snapshots, full scrollback dumps, and a scrollback-clear convenience.
Per-line ISO-8601 timestamps with millisecond precision for correlation
with Burp, responder, SIEM, etc.

## Parity with tmux-logging

| `tmux-logging` feature                        | This plugin                            |
| --------------------------------------------- | -------------------------------------- |
| Toggle continuous logging                     | `toggle` pipe message                  |
| Save visible viewport (screen capture)        | `snapshot` pipe message                |
| Save complete history                         | `dump_full` pipe message               |
| Clear pane history                            | `clear_history` pipe (opt-in)          |
| `@logging-path`                               | `output_dir` config                    |
| `@logging-filename`                           | `filename_template` config             |
| Strip ANSI via external `ansifilter`          | Built-in `strip_ansi` config           |

## Requirements

- **Zellij 0.44.2** (or whatever version `zellij-tile` is pinned to in
  `Cargo.toml`). This plugin depends on the `ReadPaneContents` permission
  and the `PaneRenderReport` event added in
  [zellij-org/zellij#4465](https://github.com/zellij-org/zellij/pull/4465),
  which require Zellij 0.44.0 or newer. The plugin ABI is auto-generated and
  changes between point releases, so the `zellij-tile` version in
  `Cargo.toml` must match your Zellij version exactly. If you upgrade Zellij,
  bump the pin and rebuild (see Troubleshooting).
- A Rust toolchain with the `wasm32-wasip1` target if you're building from source.

## Install

### From source

```bash
rustup target add wasm32-wasip1
git clone https://github.com/tzzek/zellij-logging
cd zellij-logging
cargo build --release --target wasm32-wasip1
mkdir -p ~/.config/zellij/plugins
cp target/wasm32-wasip1/release/zellij_logging.wasm ~/.config/zellij/plugins/
```

### Configure

Add the plugin block and the recommended keybindings to
`~/.config/zellij/config.kdl`. A drop-in fragment is in
[`examples/config.kdl`](examples/config.kdl); you can `include` it or paste
the contents into your config.

Minimal example:

```kdl
plugins {
    zellij-logging location="file:~/.config/zellij/plugins/zellij_logging.wasm" {
        // All keys are optional; defaults shown below.
        output_dir         "/host/zellij-logs"
        filename_template  "{date}/{session}-{pane_id}-{ts}.log"
        timestamp_lines    true
        strip_ansi         true
        auto_start         false
    }
}

keybinds {
    shared_except "locked" {
        bind "Ctrl Shift p" {
            MessagePlugin "zellij-logging" { name "toggle"; }
        }
        bind "Alt p" {
            MessagePlugin "zellij-logging" { name "snapshot"; }
        }
        bind "Alt Shift p" {
            MessagePlugin "zellij-logging" { name "dump_full"; }
        }
    }
}
```

After editing the config, restart Zellij or run `zellij action launch-or-focus-plugin`.

## Usage

| Keybind          | Pipe message     | What it does                                                                 |
| ---------------- | ---------------- | ---------------------------------------------------------------------------- |
| `Ctrl+Shift+P`   | `toggle`         | Start or stop continuous logging for the focused pane.                       |
| `Alt+P`          | `snapshot`       | Write the current viewport (visible text) to a one-shot file.                |
| `Alt+Shift+P`    | `dump_full`      | Write the entire scrollback (above + viewport + below) to a one-shot file.  |
| `Alt+C`          | `clear_history`  | Clear the focused pane's scrollback. Requires `enable_clear_history true`.  |

Continuous logging works like this on each `PaneRenderReport`:

1. Synchronously fetch the **full scrollback** for every tracked pane via
   `get_pane_scrollback(pane_id, get_full_scrollback=true)`. This returns
   `lines_above_viewport ++ viewport`: every line that has ever been on
   the pane plus what's currently visible.
2. Diff against the last full content we stored for that pane (the "baseline"
   captured at the moment the user toggled logging on).
3. Append the new tail to the log file with optional per-line timestamps.

This is intentionally similar to how `tmux-logging`'s `pipe-pane` works:
the log starts from the moment you toggle on and contains every line the
pane subsequently produces, including content that scrolled past the
viewport too fast to ever be captured by a viewport-only snapshot.

### Architectural note

`tmux-logging` taps the pane's pseudo-terminal directly via `pipe-pane`, so
it captures every byte including cursor moves and partial-line overwrites.
Zellij plugins do not have access to that level of the pty stack, so this
plugin captures the **rendered** scrollback grid instead. For typical
sequential-output tooling (nmap, gobuster, hashcat, shell commands) the
two approaches produce equivalent records. For interactive TUI applications
(vim, msfconsole) tmux-logging captures cursor-level keystrokes whereas this
plugin only sees the final-rendered state of each line; for those
workloads, `script(1)` started inside the pane is the right tool.

## Configuration

| Key                 | Type    | Default                                       | Notes                                                                                       |
| ------------------- | ------- | --------------------------------------------- | ------------------------------------------------------------------------------------------- |
| `output_dir`            | path    | `/host/zellij-logs`                           | Directory the plugin writes to. Must be reachable through a Zellij WASI mount (see below). |
| `filename_template`     | string  | `{date}/{session}-{pane_id}-{ts}.log`         | Placeholders are substituted; non-placeholder text is preserved literally.                  |
| `timestamp_lines`       | bool    | `true`                                        | Prefix every captured line with `YYYY-MM-DDTHH:MM:SS.sss+ZZZZ` (millisecond precision, computed per line). Off for one-shot snapshots. |
| `strip_ansi`            | bool    | `true`                                        | Strip CSI/OSC/2-byte escapes before writing.                                                |
| `auto_start`            | bool    | `false`                                       | If true, start logging the focused pane on plugin load (after permission is granted).       |
| `enable_clear_history`  | bool    | `false`                                       | If true, exposes the `clear_history` pipe and requests `ChangeApplicationState` permission. |

### Filename template placeholders

| Placeholder    | Meaning                                                              |
| -------------- | -------------------------------------------------------------------- |
| `{session}`    | Zellij session name (or `unknown-session`).                          |
| `{tab}`        | Tab name (or `tab-N` if unnamed).                                    |
| `{pane_id}`    | Stable pane id, e.g. `terminal_42` or `plugin_7`.                    |
| `{pane_title}` | The pane's current title (sanitised for filesystem use).             |
| `{ts}`         | ISO-8601 timestamp with timezone offset, colons replaced for safety. |
| `{date}`       | Local date, `YYYY-MM-DD`.                                            |
| `{time}`       | Local time, `HH-MM-SS` (no colons; safe on Windows/SMB).             |

Unrecognised `{placeholders}` are preserved literally so typos don't silently
disappear from the filename. All path components are sanitised: anything
outside `[A-Za-z0-9._/-]` is replaced with `_`.

## File system notes

Zellij plugins run in a WASI sandbox. The plugin only sees a fixed set of
mounts:

| Mount    | Backed by                                                                                       |
| -------- | ----------------------------------------------------------------------------------------------- |
| `/host`  | The plugin's working directory (CWD of the last focused terminal, or where Zellij was started). |
| `/data`  | A per-plugin folder, created on plugin load, **deleted on plugin unload**.                      |
| `/cache` | A persistent per-plugin cache directory.                                                        |
| `/tmp`   | Standard scratch space.                                                                         |

Default `output_dir` is `/host/zellij-logs`, which means logs land in
`<launch-dir>/zellij-logs/...`. If you want logs somewhere specific, either:

1. Launch Zellij from the parent directory you want logs under, e.g.
   `cd ~ && zellij`, which makes `/host/zellij-logs` resolve to
   `~/zellij-logs`, **or**
2. Set `output_dir` explicitly in `config.kdl` to a path under one of the
   mounts above. Paths outside these mounts are not reachable by the plugin.

`~` and `$HOME` are not expanded inside the plugin, so don't use them in
`output_dir`. Use a literal path under `/host` (or `/data`, `/cache`, `/tmp`).

## Permissions

The plugin always requests:

- **`ReadPaneContents`**: required to receive `PaneRenderReport` events and
  to call `get_pane_scrollback()`. Without it, no logging happens.

Conditionally (only if `enable_clear_history true` is set in config):

- **`ChangeApplicationState`**: required by `clear_screen_for_pane_id`, the
  Zellij API used to wipe a pane's scrollback. This permission is broader
  than the clear feature alone needs (it grants pane/tab/UI control), which
  is why the feature is opt-in.

Zellij prompts the user the first time the plugin is loaded. Granted decisions
are remembered per-plugin in Zellij's permission cache.

The plugin does **not** request any of these:

- `RunCommands`, `OpenFiles`, `WriteToStdin`: it never executes commands or
  modifies pane stdin.
- `WebAccess`: no network traffic.
- `FullHdAccess`: file I/O is confined to the WASI mounts.

## Status indicator

When the plugin pane is visible (e.g. you opened it via
`MessagePlugin ... { floating true; }`), it renders a small status panel:

```
zellij-logging: 2 pane(s) tracked
output_dir: /host/zellij-logs
template:   {date}/{session}-{pane_id}-{ts}.log

active:
  terminal_3  +00:14:21  /host/zellij-logs/2026-05-04/work-terminal_3-2026-05-04T11-32-08+0200.log
  terminal_7  +00:02:55  /host/zellij-logs/2026-05-04/work-terminal_7-2026-05-04T11-43-34+0200.log

started logging terminal_7 -> ...
permission granted: ReadPaneContents
loaded; output_dir=/host/zellij-logs, template=...
```

The status panel is informational; you don't need to keep it open for logging
to work.

## Troubleshooting

**Plugin fails to load with `could not find exported function` / `failed to
load plugin from instance` in `~/.cache/zellij/.../zellij.log` or
`/tmp/zellij-*/zellij-log/zellij.log`.**

This is an ABI mismatch between the running Zellij and the `zellij-tile`
crate the plugin was built against. Zellij's plugin command and event ABI
is regenerated from `.proto` files and changes between point releases, so
the tile crate version must match the host Zellij version exactly.

To fix:

1. Find your Zellij version: `zellij --version`.
2. Edit `Cargo.toml` so `zellij-tile = "=X.Y.Z"` matches that version
   exactly (note the leading `=` to pin instead of allowing semver bumps).
3. Rebuild and reinstall:
   ```bash
   cargo update -p zellij-tile
   cargo build --release --target wasm32-wasip1
   cp target/wasm32-wasip1/release/zellij_logging.wasm \
      ~/.config/zellij/plugins/
   ```
4. Clear Zellij's plugin cache and fully restart:
   ```bash
   rm -rf ~/.cache/zellij/file:
   zellij kill-all-sessions --yes
   zellij --session logtest
   ```

If `cargo search zellij-tile` doesn't yet show a version matching your
Zellij, you may need to depend on the upstream git repo at the right tag
until a release is published:

```toml
[target.'cfg(target_family = "wasm")'.dependencies]
zellij-tile = { git = "https://github.com/zellij-org/zellij", tag = "vX.Y.Z" }
```

**No log files appear.**
- Check `~/.config/zellij/permissions.kdl` (or run the plugin once and approve
  the prompt). The plugin needs `ReadPaneContents`.
- Verify your `output_dir` resolves inside one of the WASI mounts. The
  default `/host/zellij-logs` resolves under whatever directory Zellij was
  launched from.
- Open the plugin pane in floating mode and look at the status messages.

**Logs are full of garbage / control characters.**
- Make sure `strip_ansi` is `true`.
- Some applications emit non-ANSI control sequences (cursor positioning in
  TUIs); the stripper handles standard CSI/OSC/DCS but not every weird
  vendor extension.

**Filename has weird underscores.**
- Path-hostile characters (spaces, parentheses, accents, etc.) are replaced
  with `_` to keep paths usable. Use `{pane_id}` and `{ts}` for guaranteed
  clean names; `{pane_title}` is convenient but volatile.

**Lines are duplicated when scrolling fast.**
- The continuous logger uses viewport diffing. If a render lands while a
  redraw is in progress, the diff may overlap. The `dump_full` one-shot is
  authoritative if you need a complete record at a point in time.

## Building and testing

```bash
# Unit tests (host target).
cargo test

# Release build for Zellij.
cargo build --release --target wasm32-wasip1
```

The pure-data modules (`config`, `template`, `ansi`, `tracker`) are covered
by unit tests on the host. The plugin glue (`src/plugin.rs`) is wasm-only and
gets exercised by running it inside Zellij (see `scripts/test-plugin.sh`).

## License

MIT. See [LICENSE](LICENSE).
