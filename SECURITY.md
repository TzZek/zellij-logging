# Security

## Reporting a vulnerability

If you find a security issue, please open a private security advisory on
GitHub (the "Report a vulnerability" button on the Security tab), or email
the maintainer. Please do not open a public issue for a vulnerability that
isn't already public.

## Threat model

zellij-logging runs as a WASI plugin inside a Zellij session. Its assets are:

1. **Pane content captured to log files.** This is the whole reason the plugin
   exists, and the content can include sensitive material: passwords typed
   at prompts, API tokens echoed by tools, SSH keys printed by debug output,
   command histories, etc. Treat the contents of `output_dir` like any other
   audit log: store on encrypted volumes where appropriate, restrict access,
   and rotate or destroy when the engagement ends.

2. **The plugin itself.** It runs in Zellij's WASI sandbox and only has
   access to the filesystem mounts Zellij hands it (`/host`, `/data`,
   `/cache`, `/tmp`). It cannot escape the sandbox to read or write
   arbitrary files on the host.

3. **The user's `~/.config/zellij/config.kdl`.** This is trusted input.
   Pane content (including pane titles) is **not** trusted, see below.

## Known threats and mitigations

### Path traversal via pane titles

A program running in a tracked pane can rewrite its own title at any time
using OSC 0/2 escape sequences (`printf '\x1b]2;../../../etc/passwd\x07'`).
If the user's `filename_template` includes `{pane_title}`, an attacker who
controls a tracked pane could otherwise direct log writes outside the
configured `output_dir`.

**Mitigation:** the template renderer's `sanitise()` function strips path
separators (`/`, `\`) from substituted values and collapses runs of `..`,
so substituted values can only ever contribute a single safe path component.
The literal `/` in a template (e.g. `{date}/{session}.log`) is preserved
because it is not run through `sanitise`. Tests in
`src/template.rs::tests::sanitises_path_traversal_in_pane_title` and
`sanitises_backslash_path_traversal` cover the OSC-title attack.

WASI runtimes (Wasmi in Zellij's case) additionally sandbox path resolution
to the plugin's preopened directories, so even an unsanitised value would
not let a plugin escape `/host`. The `sanitise()` filter is defense in depth.

### ANSI escape injection in logs

Pane content includes ANSI escape sequences from any program that emits
them. By default `strip_ansi = true` removes CSI / OSC / DCS sequences
from primary logs, and the optional `<basename>.clean.log` companion file
always strips ANSI regardless of config. This protects readers of the logs
from accidentally executing terminal control sequences that could move the
cursor, change the title of the reader's terminal, or invoke the
"DECRQM" / "DA" / "DSR" reply attacks on naive viewers.

If you set `strip_ansi = false` to preserve colour, treat the resulting
log file like any other untrusted byte stream: avoid `cat`-ing it directly
into a terminal you care about; pipe through `less -R` or `cat -v` or the
`.clean.log` companion instead.

### Secret leakage

Logs are written with the user's default umask (typically `0644`,
world-readable on the local system). For engagement work, redirect
`output_dir` to a directory under `~/.engagements/<client>/` with
restrictive permissions, or run on an encrypted volume.

The plugin does **not** redact passwords, tokens, or other secrets in the
captured content. Capturing everything is the design goal; redaction would
defeat the purpose of an audit log. Apply log hygiene at the storage layer.

### Filesystem race conditions

Log files are opened with `OpenOptions::new().create(true).append(true)`.
There is no TOCTOU window: the create-or-append happens atomically.
Concurrent writes from multiple plugin invocations are serialised by the
filesystem's append guarantee.

### Resource exhaustion

The plugin holds the last full scrollback for each tracked pane in memory
to compute incremental diffs. Memory usage scales with `O(panes ×
scrollback_lines)`. Zellij's default scrollback cap (typically 10000 lines)
bounds this in practice. There is no rate limit on file I/O, so a tracked
pane producing extreme amounts of output can fill disk; monitor `output_dir`
size during long engagements.

## What the plugin does NOT do

- **No network traffic.** The plugin does not request `WebAccess`.
- **No subprocess execution.** The plugin does not request `RunCommands`.
- **No stdin manipulation.** The plugin does not request `WriteToStdin`.
- **No global keystroke interception.** The plugin does not request
  `InterceptInput`. It only sees pane content via `PaneRenderReport` and
  `get_pane_scrollback`.
- **No telemetry, no auto-update, no remote configuration.**

## Dependencies

`cargo audit` reports three warnings against transitive dependencies pulled
in via `zellij-utils` (`proc-macro-error`, `atty`, `clap` derive bits).
None are exploitable in this plugin's use of them, and they will be
resolved when upstream `zellij-utils` updates the relevant deps. Direct
dependencies (`chrono`, `zellij-tile`) are clean as of the latest audit.
