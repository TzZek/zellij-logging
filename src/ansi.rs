//! Minimal ANSI escape stripper.
//!
//! We only need to recognise the sequences that show up in normal terminal
//! output: CSI (`ESC [ ... final-byte`), OSC (`ESC ] ... BEL` or `... ST`),
//! and a handful of single-character escapes. This is intentionally a small
//! hand-rolled implementation rather than a regex or full VT parser; the
//! plugin runs on every pane render so it has to be cheap.

/// Strip ANSI escape sequences from `input`, returning a new String.
///
/// Handles:
/// - CSI sequences: `ESC [ <params> <final-byte>` (final byte 0x40-0x7E).
/// - OSC sequences: `ESC ] <params> ST` where ST is BEL (0x07) or `ESC \`.
/// - DCS/SOS/PM/APC: same termination as OSC.
/// - Two-byte escapes: `ESC <byte>` for bytes outside the introducer set.
/// - Bare `ESC` at end of input is dropped.
///
/// The scanner walks bytes for the escape framing (which is pure ASCII), but
/// copies non-ESC content as substrings of the original `&str` so multi-byte
/// UTF-8 characters round-trip unchanged.
pub fn strip(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != 0x1B {
            // Copy a run of non-ESC bytes as-is, preserving UTF-8 sequences.
            // ESC (0x1B) never appears as a UTF-8 continuation byte, so it's
            // safe to bail on the next ESC and slice the original `&str`.
            let start = i;
            while i < bytes.len() && bytes[i] != 0x1B {
                i += 1;
            }
            out.push_str(&input[start..i]);
            continue;
        }
        // ESC seen. Look at the next byte.
        let Some(&next) = bytes.get(i + 1) else {
            // Trailing ESC: drop it.
            break;
        };
        match next {
            b'[' => {
                // CSI: skip params, then a single final byte in 0x40..=0x7E.
                i += 2;
                while i < bytes.len() {
                    let c = bytes[i];
                    i += 1;
                    if (0x40..=0x7E).contains(&c) {
                        break;
                    }
                }
            },
            b']' | b'P' | b'X' | b'^' | b'_' => {
                // OSC / DCS / SOS / PM / APC.
                // Terminated by BEL (0x07) or ST (`ESC \` = 0x1B 0x5C).
                i += 2;
                while i < bytes.len() {
                    let c = bytes[i];
                    if c == 0x07 {
                        i += 1;
                        break;
                    }
                    if c == 0x1B && bytes.get(i + 1) == Some(&b'\\') {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
            },
            _ => {
                // Two-byte escape (e.g. ESC = 0x1B 0x3D for keypad mode).
                // Drop both bytes.
                i += 2;
            },
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::strip;

    #[test]
    fn passes_through_plain_text() {
        assert_eq!(strip("hello world"), "hello world");
    }

    #[test]
    fn strips_color_csi() {
        assert_eq!(strip("\x1b[31mred\x1b[0m"), "red");
    }

    #[test]
    fn strips_cursor_move_csi() {
        // CSI cursor home, then text, then erase.
        assert_eq!(strip("\x1b[H\x1b[2Jhello"), "hello");
    }

    #[test]
    fn strips_osc_with_bel_terminator() {
        // OSC 0 (set window title) terminated by BEL.
        assert_eq!(strip("\x1b]0;title\x07after"), "after");
    }

    #[test]
    fn strips_osc_with_st_terminator() {
        // OSC terminated by ST (ESC \).
        assert_eq!(strip("\x1b]0;t\x1b\\after"), "after");
    }

    #[test]
    fn strips_two_byte_escape() {
        // ESC = (keypad).
        assert_eq!(strip("\x1b=text"), "text");
    }

    #[test]
    fn handles_trailing_bare_escape() {
        // Sometimes a render report can end mid-sequence; we should not panic
        // or copy the stray ESC.
        assert_eq!(strip("text\x1b"), "text");
    }

    #[test]
    fn preserves_utf8_multibyte() {
        // Non-ASCII bytes must pass through untouched.
        let s = "\u{2615} café \x1b[1mbold\x1b[0m";
        assert_eq!(strip(s), "\u{2615} café bold");
    }

    #[test]
    fn strips_complex_prompt() {
        let prompt = "\x1b[01;32muser@host\x1b[00m:\x1b[01;34m~/dev\x1b[00m$ ";
        assert_eq!(strip(prompt), "user@host:~/dev$ ");
    }
}
