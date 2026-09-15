pub fn provider_title(raw: &str, cwd: Option<&str>) -> Option<String> {
    let clean: String = raw
        .chars()
        .take(512)
        .map(|c| {
            if c.is_control() || matches!(c, '\u{2028}' | '\u{2029}') {
                ' '
            } else {
                c
            }
        })
        .collect();
    let mut title = trim_chrome(&clean);
    loop {
        let previous = title;
        for separator in [" | ", " — ", " - "] {
            if let Some((left, right)) = title.split_once(separator) {
                if decoration(trim_chrome(left), cwd) {
                    title = trim_chrome(right);
                    break;
                }
            }
            if let Some((left, right)) = title.rsplit_once(separator) {
                if decoration(trim_chrome(right), cwd) {
                    title = trim_chrome(left);
                    break;
                }
            }
        }
        if title == previous {
            return (!decoration(title, cwd)).then(|| title.to_owned());
        }
    }
}

fn is_chrome(c: char) -> bool {
    c.is_whitespace()
        || ('\u{2800}'..='\u{28ff}').contains(&c)
        || ('◐'..='◓').contains(&c)
        || matches!(
            c,
            '✳' | '✻' | '✽' | '✶' | '✢' | '·' | '●' | '◌' | '⚠' | '✓' | '\u{fe0f}' | '\u{fe0e}'
        )
}

fn trim_chrome(text: &str) -> &str {
    text.trim_matches(is_chrome)
}

/// A status word plus the cage a provider blinks it in. Codex alternates
/// `[ ! ] Action Required` and `[ . ] Action Required` once a second to
/// blink its title (codex-rs `tui/src/chatwidget/status_surfaces.rs`), so
/// recognizing one phase and not the other makes our label flicker at the
/// same rate. Trim the cage and whatever sits inside it, rather than the
/// fillings we happen to have seen.
fn trim_caged_chrome(text: &str) -> &str {
    text.trim_matches(|c: char| is_chrome(c) || matches!(c, '[' | ']' | '!' | '.' | '…'))
}

fn decoration(text: &str, cwd: Option<&str>) -> bool {
    let lower = text.to_ascii_lowercase();
    if matches!(
        trim_caged_chrome(&lower),
        "" | "codex"
            | "claude"
            | "claude code"
            | "opencode"
            | "gemini"
            | "gemini cli"
            | "new chat"
            | "new session"
            | "untitled"
            | "idle"
            | "ready"
            | "busy"
            | "working"
            | "thinking"
            | "running"
            | "waiting"
            | "done"
            | "starting"
            | "compacting"
            | "interrupted"
            | "action required"
            | "needs input"
            | "waiting for input"
            | "waiting for approval"
    ) {
        return true;
    }
    if text.starts_with(['/', '~', '\\'])
        || text
            .as_bytes()
            .get(1..3)
            .is_some_and(|s| s == b":\\" || s == b":/")
    {
        return true;
    }
    cwd.is_some_and(|cwd| {
        let cwd = cwd.trim_end_matches(['/', '\\']);
        text == cwd || cwd.rsplit(['/', '\\']).next() == Some(text)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_spinner_frames_leave_the_topic_unchanged() {
        for glyph in ['◐', '◑', '◒', '◓', '✳', '⠂', '⠐'] {
            assert_eq!(
                provider_title(&format!("{glyph} Airplane type"), None).as_deref(),
                Some("Airplane type")
            );
            assert_eq!(provider_title(&format!("{glyph} Claude Code"), None), None);
            assert_eq!(provider_title(&glyph.to_string(), None), None);
        }
    }

    /// Codex blinks its Action Required prefix between `[ ! ]` and `[ . ]`
    /// once a second. Whatever we do with one phase we must do with the
    /// other, or the label flickers at Codex's blink rate rather than
    /// holding still.
    #[test]
    fn codex_action_required_blink_phases_agree() {
        let cwd = Some("/Users/jason/repos/yicheng47");
        for suffix in ["", " | yicheng47", " | Discuss cars | yicheng47"] {
            let visible = format!("[ ! ] Action Required{suffix}");
            let hidden = format!("[ . ] Action Required{suffix}");
            assert_eq!(
                provider_title(&visible, cwd),
                provider_title(&hidden, cwd),
                "blink phases disagree for {suffix:?}"
            );
        }
        assert_eq!(provider_title("[ . ] Action Required", cwd), None);
        assert_eq!(
            provider_title("[ . ] Action Required | Discuss cars | yicheng47", cwd).as_deref(),
            Some("Discuss cars")
        );
    }

    #[test]
    fn separates_topics_from_provider_chrome() {
        let cwd = Some("/Users/jason/repos/yicheng47");
        for raw in [
            "yicheng47",
            "⠋ yicheng47",
            "Codex",
            "Thinking…",
            "⠹",
            "",
            "~/repos/runner",
            "C:\\repos\\runner",
            "Codex | working | yicheng47",
            "[ ! ] Action Required | yicheng47",
            "[ . ] Action Required | yicheng47",
            "[   ] Action Required | yicheng47",
            "⚠️ Waiting for approval",
        ] {
            assert_eq!(provider_title(raw, cwd), None, "{raw}");
        }
        for raw in [
            "Discuss cars | yicheng47",
            "⠋ Discuss cars | yicheng47",
            "Codex | Discuss cars | Ready",
            "✳ Discuss cars - yicheng47",
            "Discuss cars ⠹ | yicheng47",
            "[ ! ] Action Required | Discuss cars | yicheng47",
            "[ . ] Action Required | Discuss cars | yicheng47",
            "/Users/jason/repos/yicheng47 | Discuss cars",
        ] {
            assert_eq!(
                provider_title(raw, cwd).as_deref(),
                Some("Discuss cars"),
                "{raw}"
            );
        }
        assert_eq!(
            provider_title("Fix parser - preserve spans", cwd).as_deref(),
            Some("Fix parser - preserve spans")
        );
        assert_eq!(
            provider_title("汽车讨论 | runner", Some("C:\\repos\\runner")).as_deref(),
            Some("汽车讨论")
        );
    }
}
