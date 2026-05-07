//! Per-pane logging state and the actual file-writing logic.
//!
//! Two write paths:
//! - Continuous: viewport snapshots from `Event::PaneRenderReport`. We diff
//!   against the previous snapshot and append only the lines that scrolled in.
//! - One-shot: full viewport or full scrollback, fetched synchronously via
//!   `get_pane_scrollback()`. Writes a self-contained snapshot file.
//!
//! Both paths share the same path rendering (`render_path`) and write helper
//! (`append_block`).

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::Local;

use crate::ansi;
use crate::config::PluginConfig;
use crate::template::TemplateContext;

/// Bookkeeping for a single pane that is being continuously logged.
pub struct TrackedPane {
    pub log_path: PathBuf,
    /// The last viewport we saw, so we can compute a diff and only append new
    /// lines instead of re-writing the whole viewport on every render report.
    pub last_viewport: Vec<String>,
    pub started_at: chrono::DateTime<chrono::Local>,
}

/// A small façade around the per-pane map and the resolved config.
///
/// Panes are keyed by a string id (whatever the caller decides; typically the
/// `Display` impl of `zellij_tile::prelude::PaneId`, e.g. `terminal_42`).
/// Keeping the tracker free of zellij types lets the unit tests run on the
/// host target without pulling in the WASI host imports.
#[derive(Default)]
pub struct Tracker {
    panes: HashMap<String, TrackedPane>,
}

impl Tracker {
    pub fn is_tracking(&self, pane: &str) -> bool {
        self.panes.contains_key(pane)
    }

    pub fn tracked_panes(&self) -> impl Iterator<Item = (&str, &TrackedPane)> {
        self.panes.iter().map(|(k, v)| (k.as_str(), v))
    }

    pub fn count(&self) -> usize {
        self.panes.len()
    }

    /// Start tracking the pane identified by `pane`. Returns the resolved log path.
    pub fn start(
        &mut self,
        pane: String,
        config: &PluginConfig,
        meta: &PaneMeta,
    ) -> Result<PathBuf, String> {
        let log_path = render_path(config, meta);
        if let Some(parent) = log_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("create_dir_all {}: {e}", parent.display()))?;
        }
        // Touch the file with a header so the user can confirm logging started.
        let header = format!(
            "# zellij-logging started {} for pane {}\n",
            Local::now().format("%Y-%m-%dT%H:%M:%S%:z"),
            meta.pane_id_str
        );
        append_block(&log_path, &header)
            .map_err(|e| format!("write header to {}: {e}", log_path.display()))?;
        self.panes.insert(
            pane,
            TrackedPane {
                log_path: log_path.clone(),
                last_viewport: Vec::new(),
                started_at: Local::now(),
            },
        );
        Ok(log_path)
    }

    /// Stop tracking `pane`. Returns the path that was being written, if any,
    /// so the caller can report it.
    pub fn stop(&mut self, pane: &str) -> Option<PathBuf> {
        let removed = self.panes.remove(pane)?;
        let footer = format!(
            "# zellij-logging stopped {}\n",
            Local::now().format("%Y-%m-%dT%H:%M:%S%:z")
        );
        let _ = append_block(&removed.log_path, &footer);
        Some(removed.log_path)
    }

    /// Forget the last-seen viewport for `pane`. Call this after issuing a
    /// pane-clear so the next render report is diffed against an empty
    /// baseline instead of producing a flood of fake "scrolled-out" lines.
    /// No-op if the pane is not tracked.
    pub fn reset_viewport(&mut self, pane: &str) {
        if let Some(tp) = self.panes.get_mut(pane) {
            tp.last_viewport.clear();
        }
    }

    /// Apply a render-report viewport to a tracked pane: diff and append.
    /// No-op if the pane is not tracked.
    pub fn on_render_report(
        &mut self,
        pane: &str,
        viewport: &[String],
        config: &PluginConfig,
    ) -> Result<(), String> {
        let Some(tracked) = self.panes.get_mut(pane) else {
            return Ok(());
        };
        let new_lines = diff_new_lines(&tracked.last_viewport, viewport);
        if new_lines.is_empty() {
            // Even if no new lines, capture the latest viewport for next diff.
            tracked.last_viewport = viewport.to_vec();
            return Ok(());
        }
        let block = format_lines(new_lines.iter().copied(), config);
        append_block(&tracked.log_path, &block)
            .map_err(|e| format!("append to {}: {e}", tracked.log_path.display()))?;
        tracked.last_viewport = viewport.to_vec();
        Ok(())
    }

    /// Write a one-shot snapshot file containing `lines`. Used for `snapshot`
    /// (visible viewport) and `dump_full` (entire scrollback).
    pub fn write_oneshot(
        &self,
        config: &PluginConfig,
        meta: &PaneMeta,
        kind: SnapshotKind,
        lines: &[String],
    ) -> Result<PathBuf, String> {
        // Decorate the template with a kind suffix so a snapshot doesn't
        // collide with a continuous log written for the same pane in the same
        // second.
        let path = render_path_with_suffix(config, meta, kind.suffix());
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("create_dir_all {}: {e}", parent.display()))?;
        }
        let header = format!(
            "# zellij-logging {} {} pane={}\n",
            kind.label(),
            Local::now().format("%Y-%m-%dT%H:%M:%S%:z"),
            meta.pane_id_str,
        );
        // For one-shots, default to NO per-line timestamps (matching tmux-logging).
        let mut snapshot_cfg = config.clone();
        snapshot_cfg.timestamp_lines = false;
        let body = format_lines(lines.iter().map(String::as_str), &snapshot_cfg);
        append_block(&path, &header)
            .map_err(|e| format!("write header to {}: {e}", path.display()))?;
        append_block(&path, &body)
            .map_err(|e| format!("write body to {}: {e}", path.display()))?;
        Ok(path)
    }
}

#[derive(Clone, Copy)]
pub enum SnapshotKind {
    Visible,
    Full,
}

impl SnapshotKind {
    fn suffix(self) -> &'static str {
        match self {
            SnapshotKind::Visible => ".visible",
            SnapshotKind::Full => ".full",
        }
    }
    fn label(self) -> &'static str {
        match self {
            SnapshotKind::Visible => "visible-snapshot",
            SnapshotKind::Full => "full-scrollback",
        }
    }
}

/// Inputs needed to build a per-pane log filename.
pub struct PaneMeta<'a> {
    pub session: &'a str,
    pub tab: &'a str,
    pub pane_id_str: String,
    pub pane_title: &'a str,
}

fn render_path(config: &PluginConfig, meta: &PaneMeta) -> PathBuf {
    let now = Local::now();
    let ctx = TemplateContext {
        session: meta.session,
        tab: meta.tab,
        pane_id: &meta.pane_id_str,
        pane_title: meta.pane_title,
        now,
    };
    let rendered = ctx.render(&config.filename_template);
    config.output_dir.join(rendered)
}

fn render_path_with_suffix(config: &PluginConfig, meta: &PaneMeta, suffix: &str) -> PathBuf {
    let mut p = render_path(config, meta);
    // Insert suffix before the final extension if any: `foo.log` → `foo.full.log`.
    let new_name = match (p.file_stem(), p.extension()) {
        (Some(stem), Some(ext)) => format!(
            "{}{}.{}",
            stem.to_string_lossy(),
            suffix,
            ext.to_string_lossy()
        ),
        (Some(stem), None) => format!("{}{}", stem.to_string_lossy(), suffix),
        _ => format!("snapshot{suffix}"),
    };
    p.set_file_name(new_name);
    p
}

/// Find the new lines in `curr` that were not in `prev`. We look for the
/// largest k such that `prev[len_prev - k..]` == `curr[..k]`; any lines after
/// that overlap are new. If there is no overlap, the whole new viewport is
/// considered new (handles screen clears and resizes).
fn diff_new_lines<'a>(prev: &[String], curr: &'a [String]) -> Vec<&'a str> {
    if prev.is_empty() {
        return curr.iter().map(String::as_str).collect();
    }
    if prev == curr {
        return Vec::new();
    }
    let max_k = prev.len().min(curr.len());
    let mut k = max_k;
    while k > 0 {
        if prev[prev.len() - k..] == curr[..k] {
            break;
        }
        k -= 1;
    }
    curr[k..].iter().map(String::as_str).collect()
}

/// ISO-8601 timestamp format used for per-line prefixes. Millisecond
/// precision so log entries from the same render batch don't all collapse
/// onto the same second, and so logs correlate cleanly with tools that emit
/// sub-second timestamps (Burp, responder, etc.).
const LINE_TIMESTAMP_FMT: &str = "%Y-%m-%dT%H:%M:%S%.3f%:z";

/// Format an iterator of lines into a single string blob according to config:
/// optional ANSI strip, optional timestamp prefix, trailing-whitespace trim.
///
/// When timestamp prefixes are enabled, each non-blank line gets a fresh
/// `Local::now()` so the timestamps reflect the actual write moment per line
/// rather than a single shared "batch" stamp.
fn format_lines<'a, I>(lines: I, config: &PluginConfig) -> String
where
    I: Iterator<Item = &'a str>,
{
    let mut out = String::new();
    for line in lines {
        let line = line.trim_end();
        if line.is_empty() {
            // Preserve blank lines so paragraph breaks survive, but don't
            // bother timestamping them: a stamp on an empty line is noise.
            out.push('\n');
            continue;
        }
        let cleaned: String = if config.strip_ansi {
            ansi::strip(line)
        } else {
            line.to_owned()
        };
        if config.timestamp_lines {
            // Per-line `now` so each line's stamp reflects when it was
            // written, not a single batch stamp.
            out.push_str(&Local::now().format(LINE_TIMESTAMP_FMT).to_string());
            out.push(' ');
        }
        out.push_str(&cleaned);
        out.push('\n');
    }
    out
}

fn append_block(path: &Path, data: &str) -> std::io::Result<()> {
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    f.write_all(data.as_bytes())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(strs: &[&str]) -> Vec<String> {
        strs.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn diff_first_render_is_all_new() {
        let prev: Vec<String> = vec![];
        let curr = s(&["a", "b", "c"]);
        assert_eq!(diff_new_lines(&prev, &curr), vec!["a", "b", "c"]);
    }

    #[test]
    fn diff_no_change_is_empty() {
        let prev = s(&["a", "b", "c"]);
        let curr = s(&["a", "b", "c"]);
        assert!(diff_new_lines(&prev, &curr).is_empty());
    }

    #[test]
    fn diff_scroll_by_one_appends_one_line() {
        let prev = s(&["a", "b", "c", "d"]);
        let curr = s(&["b", "c", "d", "e"]);
        assert_eq!(diff_new_lines(&prev, &curr), vec!["e"]);
    }

    #[test]
    fn diff_scroll_by_two_appends_two_lines() {
        let prev = s(&["a", "b", "c", "d"]);
        let curr = s(&["c", "d", "e", "f"]);
        assert_eq!(diff_new_lines(&prev, &curr), vec!["e", "f"]);
    }

    #[test]
    fn diff_no_overlap_dumps_whole_viewport() {
        // Screen cleared: nothing in curr matches prev.
        let prev = s(&["a", "b", "c"]);
        let curr = s(&["x", "y", "z"]);
        assert_eq!(diff_new_lines(&prev, &curr), vec!["x", "y", "z"]);
    }

    #[test]
    fn diff_resize_smaller_viewport() {
        // Pane shrinks; we lose context, dump whole viewport.
        let prev = s(&["a", "b", "c", "d", "e"]);
        let curr = s(&["c", "d", "e"]);
        // Largest k where prev[end-k..] == curr[..k]: k=3 → prev[2..5]=[c,d,e].
        // No new lines.
        assert!(diff_new_lines(&prev, &curr).is_empty());
    }

    #[test]
    fn format_strips_ansi_and_adds_timestamp() {
        let cfg = PluginConfig {
            timestamp_lines: true,
            strip_ansi: true,
            ..PluginConfig::default()
        };
        let out = format_lines(["\x1b[31mhello\x1b[0m"].into_iter(), &cfg);
        assert!(out.contains("hello"));
        assert!(!out.contains("\x1b"));
        // ISO timestamp prefix sanity check: starts with year.
        assert!(
            out.starts_with(&format!("{}", Local::now().format("%Y"))),
            "timestamp prefix missing: {out}"
        );
    }

    #[test]
    fn format_timestamp_has_millisecond_precision() {
        let cfg = PluginConfig {
            timestamp_lines: true,
            strip_ansi: false,
            ..PluginConfig::default()
        };
        let out = format_lines(["hello"].into_iter(), &cfg);
        // Expect "YYYY-MM-DDTHH:MM:SS.NNN+HHMM hello\n", so the first 23
        // chars after the date are the time including `.NNN`. Look for the
        // millisecond decimal point at the right column.
        // Format: 2026-05-04T14:30:45.123+02:00
        //         ^^^^^^^^^^^^^^^^^^^^ ^
        //         0                  19 20 (the dot)
        let dot_idx = 19;
        assert_eq!(
            &out[dot_idx..dot_idx + 1],
            ".",
            "no millisecond separator in {out}"
        );
        // Three digits after the dot.
        assert!(
            out[dot_idx + 1..dot_idx + 4].chars().all(|c| c.is_ascii_digit()),
            "milliseconds not three digits in {out}"
        );
    }

    #[test]
    fn format_preserves_ansi_when_disabled() {
        let cfg = PluginConfig {
            timestamp_lines: false,
            strip_ansi: false,
            ..PluginConfig::default()
        };
        let out = format_lines(["\x1b[31mhello\x1b[0m"].into_iter(), &cfg);
        assert_eq!(out, "\x1b[31mhello\x1b[0m\n");
    }

    #[test]
    fn format_trims_trailing_whitespace() {
        let cfg = PluginConfig {
            timestamp_lines: false,
            strip_ansi: false,
            ..PluginConfig::default()
        };
        let out = format_lines(["hello       ", "world"].into_iter(), &cfg);
        assert_eq!(out, "hello\nworld\n");
    }

    #[test]
    fn render_path_with_suffix_inserts_before_extension() {
        let cfg = PluginConfig {
            output_dir: PathBuf::from("/tmp/logs"),
            filename_template: "session.log".to_owned(),
            ..PluginConfig::default()
        };
        let meta = PaneMeta {
            session: "s",
            tab: "t",
            pane_id_str: "terminal_1".to_owned(),
            pane_title: "p",
        };
        let path = render_path_with_suffix(&cfg, &meta, ".full");
        assert_eq!(path, PathBuf::from("/tmp/logs/session.full.log"));
    }
}
