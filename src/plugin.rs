//! WASI-only plugin glue: the `State` struct and its `ZellijPlugin` impl.
//! This module is only compiled on `target_family = "wasm"`; on the host
//! target, the `zellij-tile` crate is absent and all of this is skipped.
//!
//! The `register_plugin!(State)` macro invocation lives in `main.rs` (the
//! crate root) instead of here, because the macro generates a top-level
//! `fn main()` that becomes the WASI `_start` entry point. That entry has
//! to be at the crate root for rustc to wire it up correctly.

use std::collections::BTreeMap;

use zellij_tile::prelude::*;

use crate::config::PluginConfig;
use crate::tracker::{PaneMeta, SnapshotKind, Tracker};

#[derive(Default)]
pub struct State {
    config: PluginConfig,
    tracker: Tracker,
    /// Most recent session name we observed via SessionUpdate. Used to fill
    /// `{session}` in filename templates. Empty until the first SessionUpdate.
    session_name: String,
    /// Most recent set of tabs, used to look up tab names by index.
    tabs: Vec<TabInfo>,
    /// Most recent pane manifest, used to look up pane title by id.
    panes: PaneManifest,
    /// Whether the plugin has been granted the `ReadPaneContents` permission.
    /// We don't try to write logs until this is true: subscribing to render
    /// reports without the permission would just produce empty events.
    permitted: bool,
    /// Status messages we render in the plugin pane (most recent first).
    status: Vec<String>,
}

impl ZellijPlugin for State {
    fn load(&mut self, configuration: BTreeMap<String, String>) {
        self.config = PluginConfig::from_map(&configuration);
        // - ReadPaneContents: required for PaneRenderReport events and
        //   get_pane_scrollback() (the core logging capability).
        // - ReadApplicationState: required for SessionUpdate / TabUpdate /
        //   PaneUpdate events, which the plugin uses to resolve the
        //   {session}, {tab}, and {pane_title} placeholders in filename
        //   templates. Without it the plugin still logs, but to filenames
        //   like `unknown-session-tab-pane.log` instead of meaningful ones.
        // - ReadCliPipes: required for cli_pipe_output() and
        //   unblock_cli_pipe_input(). Without it, `zellij pipe ... --name X`
        //   blocks forever because the plugin can't tell Zellij the pipe is
        //   done.
        // - ChangeApplicationState: required by `clear_history` (which
        //   wipes a pane's scrollback) and by `visual_indicator` (which
        //   highlights tracked panes). The permission is broader than
        //   either feature alone needs, so we only request it when at
        //   least one of those features is enabled.
        let mut perms = vec![
            PermissionType::ReadPaneContents,
            PermissionType::ReadApplicationState,
            PermissionType::ReadCliPipes,
        ];
        if self.config.enable_clear_history || self.config.visual_indicator {
            perms.push(PermissionType::ChangeApplicationState);
        }
        request_permission(&perms);
        subscribe(&[
            EventType::PaneRenderReport,
            EventType::SessionUpdate,
            EventType::TabUpdate,
            EventType::PaneUpdate,
            EventType::PermissionRequestResult,
        ]);
        self.push_status(format!(
            "loaded; output_dir={}, template={}",
            self.config.output_dir.display(),
            self.config.filename_template
        ));
        if self.config.enable_clear_history {
            self.push_status("clear_history pipe enabled".to_owned());
        }
    }

    fn update(&mut self, event: Event) -> bool {
        match event {
            Event::PermissionRequestResult(status) => {
                self.permitted = matches!(status, PermissionStatus::Granted);
                if self.permitted {
                    self.push_status("permission granted: ReadPaneContents".to_owned());
                    if self.config.auto_start {
                        // Best-effort: start tracking the focused pane so the
                        // user gets immediate logging without a keypress.
                        let _ = self.toggle_focused();
                    }
                } else {
                    self.push_status("permission denied: ReadPaneContents".to_owned());
                }
                true
            },
            Event::SessionUpdate(sessions, _) => {
                if let Some(s) = sessions.iter().find(|s| s.is_current_session) {
                    self.session_name = s.name.clone();
                }
                false
            },
            Event::TabUpdate(tabs) => {
                self.tabs = tabs;
                false
            },
            Event::PaneUpdate(panes) => {
                self.panes = panes;
                false
            },
            Event::PaneRenderReport(reports) => {
                if !self.permitted {
                    return false;
                }
                let cfg = self.config.clone();
                let mut errors: Vec<String> = Vec::new();
                // The render report tells us which panes re-rendered, but the
                // viewport it carries is only the visible window. To capture
                // every line that ever passed through the pane (including
                // content that scrolled past too fast for the viewport to
                // catch), we synchronously fetch the full scrollback for each
                // tracked pane and feed that to the tracker for diffing.
                for pane in reports.keys() {
                    let key = format!("{pane}");
                    if !self.tracker.is_tracking(&key) {
                        continue;
                    }
                    match get_pane_scrollback(*pane, true) {
                        Ok(contents) => {
                            let full = build_full_content(&contents);
                            if let Err(e) =
                                self.tracker.on_content_update(&key, &full, &cfg)
                            {
                                errors.push(format!("write error: {e}"));
                            }
                        },
                        Err(e) => {
                            errors.push(format!("scrollback fetch failed for {pane}: {e}"));
                        },
                    }
                }
                let mut should_render = false;
                for e in errors {
                    self.push_status(e);
                    should_render = true;
                }
                should_render
            },
            _ => false,
        }
    }

    fn pipe(&mut self, pipe_message: PipeMessage) -> bool {
        // Don't gate on `self.permitted` here. PermissionRequestResult is
        // delivered asynchronously, so a pipe can arrive before the flag is
        // set (especially on fresh sessions where the keybind/CLI fires
        // immediately after plugin load). Zellij enforces permissions at
        // the host call sites, and the inner handlers convert any denials
        // into Err(...) which we surface to the user.
        let reply: String = {
            match pipe_message.name.as_str() {
                "toggle" => match self.toggle_focused() {
                    Ok(msg) => {
                        self.push_status(msg.clone());
                        msg
                    },
                    Err(msg) => {
                        let line = format!("toggle failed: {msg}");
                        self.push_status(line.clone());
                        line
                    },
                },
                "snapshot" => match self.snapshot_focused(SnapshotKind::Visible) {
                    Ok(path) => {
                        let line = format!("snapshot written: {}", path.display());
                        self.push_status(line.clone());
                        line
                    },
                    Err(msg) => {
                        let line = format!("snapshot failed: {msg}");
                        self.push_status(line.clone());
                        line
                    },
                },
                "dump_full" => match self.snapshot_focused(SnapshotKind::Full) {
                    Ok(path) => {
                        let line = format!("full dump written: {}", path.display());
                        self.push_status(line.clone());
                        line
                    },
                    Err(msg) => {
                        let line = format!("dump_full failed: {msg}");
                        self.push_status(line.clone());
                        line
                    },
                },
                "clear_history" => match self.clear_focused_history() {
                    Ok(msg) => {
                        self.push_status(msg.clone());
                        msg
                    },
                    Err(msg) => {
                        let line = format!("clear_history failed: {msg}");
                        self.push_status(line.clone());
                        line
                    },
                },
                other => {
                    let line = format!("unknown pipe message: {other}");
                    self.push_status(line.clone());
                    line
                },
            }
        };

        // If the pipe came from `zellij pipe` on the CLI, the CLI is blocked
        // waiting for our response. Send the reply back as the pipe's
        // output and explicitly unblock so the CLI sees EOF and exits.
        // Without this, `zellij pipe ... --name toggle` hangs indefinitely.
        if let PipeSource::Cli(pipe_id) = &pipe_message.source {
            cli_pipe_output(pipe_id, &format!("{reply}\n"));
            unblock_cli_pipe_input(pipe_id);
        }
        true
    }

    fn render(&mut self, rows: usize, _cols: usize) {
        let active = self.tracker.count();
        println!("zellij-logging: {active} pane(s) tracked");
        println!("output_dir: {}", self.config.output_dir.display());
        println!("template:   {}", self.config.filename_template);
        println!();
        let mut used = 4;

        if active > 0 {
            println!("active:");
            used += 1;
            let now = chrono::Local::now();
            for (pane, tp) in self.tracker.tracked_panes() {
                let elapsed = now.signed_duration_since(tp.started_at);
                let secs = elapsed.num_seconds().max(0);
                println!(
                    "  {pane}  +{:02}:{:02}:{:02}  {}",
                    secs / 3600,
                    (secs / 60) % 60,
                    secs % 60,
                    tp.log_path.display(),
                );
                used += 1;
                if used >= rows {
                    return;
                }
            }
            println!();
            used += 1;
        }

        let available = rows.saturating_sub(used);
        for line in self.status.iter().take(available) {
            println!("{line}");
        }
    }
}

impl State {
    fn push_status(&mut self, msg: String) {
        self.status.insert(0, msg);
        const MAX_STATUS: usize = 64;
        if self.status.len() > MAX_STATUS {
            self.status.truncate(MAX_STATUS);
        }
    }

    /// Look up the focused pane id and assemble the metadata we need to
    /// render filename templates. Errors come back as user-facing strings.
    fn focused_meta(&self) -> Result<(PaneId, OwnedPaneMeta), String> {
        let (tab_index, pane_id) = get_focused_pane_info()
            .map_err(|e| format!("get_focused_pane_info: {e}"))?;
        let tab_name = self
            .tabs
            .iter()
            .find(|t| t.position == tab_index)
            .map(|t| {
                if t.name.is_empty() {
                    format!("tab-{}", t.position)
                } else {
                    t.name.clone()
                }
            })
            .unwrap_or_else(|| format!("tab-{tab_index}"));
        let pane_title = self
            .panes
            .panes
            .get(&tab_index)
            .and_then(|panes| {
                panes
                    .iter()
                    .find(|p| matches_pane_id(p, &pane_id))
                    .map(|p| p.title.clone())
            })
            .unwrap_or_else(|| "pane".to_owned());
        let session = if self.session_name.is_empty() {
            "unknown-session".to_owned()
        } else {
            self.session_name.clone()
        };
        Ok((
            pane_id,
            OwnedPaneMeta {
                session,
                tab: tab_name,
                pane_id_str: format!("{pane_id}"),
                pane_title,
            },
        ))
    }

    fn toggle_focused(&mut self) -> Result<String, String> {
        let (pane_id, owned) = self.focused_meta()?;
        let key = format!("{pane_id}");
        if self.tracker.is_tracking(&key) {
            let path = self
                .tracker
                .stop(&key)
                .ok_or_else(|| "pane was not tracked".to_owned())?;
            if self.config.visual_indicator {
                highlight_and_unhighlight_panes(vec![], vec![pane_id]);
            }
            return Ok(format!("stopped logging {pane_id} ({})", path.display()));
        }
        // Capture the current full scrollback as the baseline so the log only
        // contains content produced from now on, matching tmux-logging's
        // pipe-pane behaviour. If the fetch fails (no permission etc.) we
        // surface the error without inserting a half-tracked entry.
        let baseline = match get_pane_scrollback(pane_id, true) {
            Ok(contents) => build_full_content(&contents),
            Err(e) => return Err(format!("get_pane_scrollback: {e}")),
        };
        let meta = owned.as_ref();
        let path = self
            .tracker
            .start(key, &self.config, &meta, baseline)?;
        if self.config.visual_indicator {
            highlight_and_unhighlight_panes(vec![pane_id], vec![]);
        }
        Ok(format!("started logging {pane_id} -> {}", path.display()))
    }

    fn clear_focused_history(&mut self) -> Result<String, String> {
        if !self.config.enable_clear_history {
            return Err(
                "clear_history is disabled; set enable_clear_history true in plugin config"
                    .to_owned(),
            );
        }
        let (pane_id, _) = self.focused_meta()?;
        clear_screen_for_pane_id(pane_id);
        // The next update for this pane will look unrelated to the last full
        // content we have on file, so the diff would dump a wall of blank
        // lines. Forget the last content for the cleared pane so the next
        // update is treated as a fresh baseline.
        let key = format!("{pane_id}");
        self.tracker.reset_content(&key);
        Ok(format!("cleared scrollback for {pane_id}"))
    }

    fn snapshot_focused(&mut self, kind: SnapshotKind) -> Result<std::path::PathBuf, String> {
        let (pane_id, owned) = self.focused_meta()?;
        let want_full = matches!(kind, SnapshotKind::Full);
        let contents = get_pane_scrollback(pane_id, want_full)
            .map_err(|e| format!("get_pane_scrollback: {e}"))?;
        let mut lines: Vec<String> = Vec::new();
        // For Full snapshots, prepend lines_above_viewport, then viewport,
        // then lines_below_viewport. For Visible, just the viewport.
        if want_full {
            lines.extend(contents.lines_above_viewport.iter().cloned());
        }
        lines.extend(contents.viewport.iter().cloned());
        if want_full {
            lines.extend(contents.lines_below_viewport.iter().cloned());
        }
        let meta = owned.as_ref();
        self.tracker.write_oneshot(&self.config, &meta, kind, &lines)
    }
}

fn matches_pane_id(p: &PaneInfo, want: &PaneId) -> bool {
    match want {
        PaneId::Terminal(id) => !p.is_plugin && p.id == *id,
        PaneId::Plugin(id) => p.is_plugin && p.id == *id,
    }
}

/// Build the full sequential record of a pane's content from the scrollback
/// response: lines that have already scrolled past the viewport followed by
/// what's currently visible. `lines_below_viewport` (future-scroll-back
/// content for users scrolled up) is intentionally ignored, both because it
/// is rare in normal use and because it would distort the linear "what
/// happened next" record we want for an engagement log.
fn build_full_content(contents: &PaneContents) -> Vec<String> {
    let mut full = Vec::with_capacity(
        contents.lines_above_viewport.len() + contents.viewport.len(),
    );
    full.extend(contents.lines_above_viewport.iter().cloned());
    full.extend(contents.viewport.iter().cloned());
    full
}

/// Owned counterpart of `PaneMeta`. The tracker takes a borrowed view; we
/// keep the strings alive on the stack long enough to make the call.
struct OwnedPaneMeta {
    session: String,
    tab: String,
    pane_id_str: String,
    pane_title: String,
}

impl OwnedPaneMeta {
    fn as_ref(&self) -> PaneMeta<'_> {
        PaneMeta {
            session: &self.session,
            tab: &self.tab,
            pane_id_str: self.pane_id_str.clone(),
            pane_title: &self.pane_title,
        }
    }
}
