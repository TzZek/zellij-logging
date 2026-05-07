//! WASI-only plugin glue: the `State` struct, the `ZellijPlugin` impl, and
//! the `register_plugin!` macro invocation. This module is only compiled on
//! `target_family = "wasm"`; on the host target, the `zellij-tile` crate is
//! absent and all of this is skipped.

use std::collections::BTreeMap;

use zellij_tile::prelude::*;

use crate::config::PluginConfig;
use crate::tracker::{PaneMeta, SnapshotKind, Tracker};

#[derive(Default)]
struct State {
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
    /// Whether the user has been warned about a missing permission. Avoid
    /// spamming the same warning on every pipe message.
    warned_about_permission: bool,
}

register_plugin!(State);

impl ZellijPlugin for State {
    fn load(&mut self, configuration: BTreeMap<String, String>) {
        self.config = PluginConfig::from_map(&configuration);
        request_permission(&[PermissionType::ReadPaneContents]);
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
                for (pane, contents) in reports.iter() {
                    let key = format!("{pane}");
                    if !self.tracker.is_tracking(&key) {
                        continue;
                    }
                    if let Err(e) =
                        self.tracker.on_render_report(&key, &contents.viewport, &cfg)
                    {
                        errors.push(format!("write error: {e}"));
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
        if !self.permitted && !self.warned_about_permission {
            self.push_status(
                "ReadPaneContents not yet granted; ignoring pipe until permission resolves"
                    .to_owned(),
            );
            self.warned_about_permission = true;
            return true;
        }
        if !self.permitted {
            return false;
        }
        match pipe_message.name.as_str() {
            "toggle" => match self.toggle_focused() {
                Ok(msg) => self.push_status(msg),
                Err(msg) => self.push_status(format!("toggle failed: {msg}")),
            },
            "snapshot" => match self.snapshot_focused(SnapshotKind::Visible) {
                Ok(path) => self.push_status(format!("snapshot written: {}", path.display())),
                Err(msg) => self.push_status(format!("snapshot failed: {msg}")),
            },
            "dump_full" => match self.snapshot_focused(SnapshotKind::Full) {
                Ok(path) => self.push_status(format!("full dump written: {}", path.display())),
                Err(msg) => self.push_status(format!("dump_full failed: {msg}")),
            },
            other => {
                self.push_status(format!("unknown pipe message: {other}"));
            },
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
            return Ok(format!("stopped logging {pane_id} ({})", path.display()));
        }
        let meta = owned.as_ref();
        let path = self.tracker.start(key, &self.config, &meta)?;
        Ok(format!("started logging {pane_id} -> {}", path.display()))
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
