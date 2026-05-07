//! Plugin configuration parsed from the `BTreeMap<String, String>` Zellij
//! hands us in `load()`.
//!
//! Configuration keys (all optional):
//! - `output_dir`            where to write logs. Defaults to `/host/zellij-logs`.
//! - `filename_template`     see `template` module. Defaults to a sensible per-pane name.
//! - `timestamp_lines`       bool, prefix each captured line with an ISO timestamp.
//!                           Defaults to `true` for continuous logging.
//! - `strip_ansi`            bool, strip ANSI escapes before writing. Defaults `true`.
//! - `auto_start`            bool, start logging every focused pane on plugin load.
//!                           Defaults `false`. Off by default because turning it
//!                           on in shared sessions could surprise other clients.
//! - `enable_clear_history`  bool, expose the `clear_history` pipe message
//!                           that wipes the focused pane's scrollback. Off by
//!                           default because it requires `ChangeApplicationState`,
//!                           which broadens the plugin's permission scope.

use std::collections::BTreeMap;
use std::path::PathBuf;

const DEFAULT_OUTPUT_DIR: &str = "/host/zellij-logs";
const DEFAULT_TEMPLATE: &str = "{date}/{session}-{pane_id}-{ts}.log";

#[derive(Debug, Clone)]
pub struct PluginConfig {
    pub output_dir: PathBuf,
    pub filename_template: String,
    pub timestamp_lines: bool,
    pub strip_ansi: bool,
    pub auto_start: bool,
    pub enable_clear_history: bool,
}

impl Default for PluginConfig {
    fn default() -> Self {
        Self {
            output_dir: PathBuf::from(DEFAULT_OUTPUT_DIR),
            filename_template: DEFAULT_TEMPLATE.to_owned(),
            timestamp_lines: true,
            strip_ansi: true,
            auto_start: false,
            enable_clear_history: false,
        }
    }
}

impl PluginConfig {
    /// Build a config from the key/value map Zellij passes to `load()`.
    /// Unknown keys are ignored. Bad bools fall back to the default.
    pub fn from_map(map: &BTreeMap<String, String>) -> Self {
        let mut cfg = Self::default();
        if let Some(v) = map.get("output_dir").map(String::as_str).filter(|s| !s.is_empty()) {
            cfg.output_dir = PathBuf::from(v);
        }
        if let Some(v) = map
            .get("filename_template")
            .map(String::as_str)
            .filter(|s| !s.is_empty())
        {
            cfg.filename_template = v.to_owned();
        }
        if let Some(v) = map.get("timestamp_lines").and_then(parse_bool) {
            cfg.timestamp_lines = v;
        }
        if let Some(v) = map.get("strip_ansi").and_then(parse_bool) {
            cfg.strip_ansi = v;
        }
        if let Some(v) = map.get("auto_start").and_then(parse_bool) {
            cfg.auto_start = v;
        }
        if let Some(v) = map.get("enable_clear_history").and_then(parse_bool) {
            cfg.enable_clear_history = v;
        }
        cfg
    }
}

fn parse_bool(value: &String) -> Option<bool> {
    match value.to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" | "1" => Some(true),
        "false" | "no" | "off" | "0" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn defaults_when_map_empty() {
        let cfg = PluginConfig::from_map(&BTreeMap::new());
        assert_eq!(cfg.output_dir, PathBuf::from("/host/zellij-logs"));
        assert!(cfg.timestamp_lines);
        assert!(cfg.strip_ansi);
        assert!(!cfg.auto_start);
        assert!(!cfg.enable_clear_history);
    }

    #[test]
    fn clear_history_opts_in() {
        let cfg = PluginConfig::from_map(&map(&[("enable_clear_history", "true")]));
        assert!(cfg.enable_clear_history);
    }

    #[test]
    fn overrides_output_dir() {
        let cfg = PluginConfig::from_map(&map(&[("output_dir", "/tmp/logs")]));
        assert_eq!(cfg.output_dir, PathBuf::from("/tmp/logs"));
    }

    #[test]
    fn parses_bools_case_insensitively() {
        let cfg = PluginConfig::from_map(&map(&[
            ("timestamp_lines", "False"),
            ("strip_ansi", "OFF"),
            ("auto_start", "yes"),
        ]));
        assert!(!cfg.timestamp_lines);
        assert!(!cfg.strip_ansi);
        assert!(cfg.auto_start);
    }

    #[test]
    fn ignores_unknown_keys_and_garbage_bools() {
        let cfg = PluginConfig::from_map(&map(&[
            ("nonsense", "value"),
            ("strip_ansi", "maybe"),
        ]));
        // garbage bool falls through to default (true).
        assert!(cfg.strip_ansi);
    }

    #[test]
    fn empty_string_keeps_defaults() {
        let cfg = PluginConfig::from_map(&map(&[
            ("output_dir", ""),
            ("filename_template", ""),
        ]));
        assert_eq!(cfg.output_dir, PathBuf::from("/host/zellij-logs"));
        assert_eq!(cfg.filename_template, "{date}/{session}-{pane_id}-{ts}.log");
    }
}
