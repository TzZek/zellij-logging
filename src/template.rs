//! Filename template rendering.
//!
//! Supports placeholders:
//! - `{session}`     session name (or `unknown-session`)
//! - `{tab}`         tab name (or `tab-N` if unnamed; `tab` if no info)
//! - `{pane_id}`     pane id, e.g. `terminal_3` or `plugin_7`
//! - `{pane_title}`  pane title (or `pane`)
//! - `{ts}`          ISO-8601 timestamp with timezone offset
//! - `{date}`        local date as `YYYY-MM-DD`
//! - `{time}`        local time as `HH-MM-SS` (filename-safe, no colons)
//!
//! Anything inside braces that is not one of these keys is left as-is, on the
//! theory that surprising silent removal is worse than a literal `{whatever}`
//! showing up in the filename.

use chrono::{DateTime, Local};

/// Inputs available to the template renderer. All fields are owned strings so
/// the renderer is independent of the rest of the plugin state.
#[derive(Debug, Clone)]
pub struct TemplateContext<'a> {
    pub session: &'a str,
    pub tab: &'a str,
    pub pane_id: &'a str,
    pub pane_title: &'a str,
    pub now: DateTime<Local>,
}

impl<'a> TemplateContext<'a> {
    /// Render `template`, substituting known placeholders and sanitising the
    /// result so it is safe to use as a path component on common filesystems.
    pub fn render(&self, template: &str) -> String {
        let mut out = String::with_capacity(template.len() + 32);
        let bytes = template.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'{' {
                if let Some(end) = find_close(bytes, i + 1) {
                    let key = &template[i + 1..end];
                    if let Some(value) = self.resolve(key) {
                        out.push_str(&sanitise(&value));
                        i = end + 1;
                        continue;
                    }
                }
            }
            // Not a recognised placeholder: copy literal byte.
            out.push(bytes[i] as char);
            i += 1;
        }
        out
    }

    fn resolve(&self, key: &str) -> Option<String> {
        match key {
            "session" => Some(self.session.to_owned()),
            "tab" => Some(self.tab.to_owned()),
            "pane_id" => Some(self.pane_id.to_owned()),
            "pane_title" => Some(self.pane_title.to_owned()),
            // Pentest-friendly default: ISO-8601 with offset, but with colons
            // replaced because Windows filesystems and SMB shares reject them.
            "ts" => Some(self.now.format("%Y-%m-%dT%H-%M-%S%z").to_string()),
            "date" => Some(self.now.format("%Y-%m-%d").to_string()),
            "time" => Some(self.now.format("%H-%M-%S").to_string()),
            _ => None,
        }
    }
}

fn find_close(bytes: &[u8], start: usize) -> Option<usize> {
    let mut i = start;
    while i < bytes.len() {
        match bytes[i] {
            b'}' => return Some(i),
            // No nested braces; bail out on the next opening brace too so we
            // don't swallow `{{` accidentally.
            b'{' => return None,
            _ => i += 1,
        }
    }
    None
}

/// Replace path-hostile characters with underscores. We keep `/` because the
/// template can legitimately produce subdirectories (e.g. `{date}/...`), and
/// we keep `-`, `_`, `.`. Everything else outside `[A-Za-z0-9._/-]` is
/// replaced. Empty results are replaced with `_` so the path component is
/// never empty.
fn sanitise(value: &str) -> String {
    let mut s: String = value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if s.is_empty() {
        s.push('_');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn ctx(now: DateTime<Local>) -> TemplateContext<'static> {
        TemplateContext {
            session: "session-A",
            tab: "tab-1",
            pane_id: "terminal_42",
            pane_title: "vim main.rs",
            now,
        }
    }

    fn fixed_now() -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 5, 4, 14, 30, 45).unwrap()
    }

    #[test]
    fn substitutes_session_and_pane_id() {
        let c = ctx(fixed_now());
        assert_eq!(c.render("{session}-{pane_id}.log"), "session-A-terminal_42.log");
    }

    #[test]
    fn renders_date_and_time() {
        let c = ctx(fixed_now());
        let out = c.render("{date}/{time}.log");
        assert_eq!(out, "2026-05-04/14-30-45.log");
    }

    #[test]
    fn ts_contains_timezone_offset() {
        let c = ctx(fixed_now());
        let out = c.render("{ts}.log");
        assert!(out.starts_with("2026-05-04T14-30-45"), "got {out}");
        assert!(out.ends_with(".log"));
        // Must end with a timezone offset. The chrono format produces
        // `+HHMM` or `-HHMM`; our path sanitiser replaces `+` with `_`
        // (since `+` isn't in the path-safe allowlist) but leaves `-`
        // alone (it is). So accept any of `+HHMM`, `-HHMM`, or `_HHMM`
        // in the last five characters of the stem.
        let stem = out.trim_end_matches(".log");
        let last5 = &stem[stem.len() - 5..];
        assert!(
            last5.starts_with('+') || last5.starts_with('-') || last5.starts_with('_'),
            "no timezone offset in {out}"
        );
        assert!(
            last5[1..].chars().all(|c| c.is_ascii_digit()),
            "timezone digits not all numeric in {out}"
        );
    }

    #[test]
    fn unknown_placeholders_are_preserved_literally() {
        let c = ctx(fixed_now());
        assert_eq!(c.render("{nope}.log"), "{nope}.log");
    }

    #[test]
    fn sanitises_pane_title_with_spaces() {
        let c = ctx(fixed_now());
        let out = c.render("{pane_title}.log");
        assert_eq!(out, "vim_main.rs.log");
    }

    #[test]
    fn sanitises_unicode_in_session_name() {
        let mut c = ctx(fixed_now());
        c.session = "résumé/café";
        let out = c.render("{session}.log");
        // / is preserved (it's a path separator); accented letters become _.
        assert_eq!(out, "r_sum_/caf_.log");
    }

    #[test]
    fn empty_value_becomes_single_underscore() {
        let mut c = ctx(fixed_now());
        c.session = "";
        let out = c.render("{session}.log");
        assert_eq!(out, "_.log");
    }

    #[test]
    fn nested_brace_is_treated_as_literal() {
        let c = ctx(fixed_now());
        // We don't support nesting, so {{x}} stays put.
        let out = c.render("{{x}}.log");
        assert_eq!(out, "{{x}}.log");
    }
}
