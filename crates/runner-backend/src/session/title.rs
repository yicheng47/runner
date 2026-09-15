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

fn trim_chrome(text: &str) -> &str {
    text.trim_matches(|c: char| {
        c.is_whitespace()
            || ('\u{2800}'..='\u{28ff}').contains(&c)
            || ('◐'..='◓').contains(&c)
            || matches!(
                c,
                '✳' | '✻' | '✽' | '✶' | '✢' | '·' | '●' | '◌' | '⚠' | '✓' | '\u{fe0f}' | '\u{fe0e}'
            )
    })
}

fn decoration(text: &str, cwd: Option<&str>) -> bool {
    let lower = text.to_ascii_lowercase();
    if matches!(
        lower
            .trim_start_matches(['[', ']', '!', ' '])
            .trim_end_matches(['.', '…']),
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

pub fn prompt_title(prompt: &str) -> Option<String> {
    let preview: String = prompt.chars().take(512).collect();
    if preview.trim_start().starts_with('/') {
        return None;
    }
    let without_urls = preview
        .trim()
        .lines()
        .next()?
        .split_whitespace()
        .filter(|word| {
            let word = word.trim_start_matches(['[', '(', '*', '_']);
            !word.starts_with("https://") && !word.starts_with("http://")
        })
        .collect::<Vec<_>>()
        .join(" ");
    let clause = without_urls
        .split(['.', '!', '?', ';', '。', '！', '？'])
        .next()?;
    let mut candidate = clause.trim();
    for _ in 0..3 {
        let lower = candidate.to_ascii_lowercase();
        if let Some(prefix) = [
            "can you please ",
            "could you please ",
            "would you please ",
            "can you ",
            "could you ",
            "would you ",
            "please ",
            "i want you to ",
            "i need you to ",
            "i want to ",
            "i need to ",
            "help me to ",
            "help me ",
            "let's ",
            "let’s ",
            "we need to ",
            "need to ",
        ]
        .into_iter()
        .find(|prefix| lower.starts_with(prefix))
        {
            candidate = candidate[prefix.len()..].trim_start();
        } else {
            break;
        }
    }
    let text: String = candidate
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c.is_whitespace() {
                c
            } else {
                ' '
            }
        })
        .collect();
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = text.chars();
    let first = chars.next()?;
    let title: String = first.to_uppercase().chain(chars).collect();
    let mut short: String = title.chars().take(40).collect();
    if title.chars().count() > 40 {
        if let Some((offset, _)) = short.char_indices().rev().find(|(_, c)| c.is_whitespace()) {
            if short[..offset].chars().count() >= 22 {
                short.truncate(offset);
            }
        }
    }
    Some(short.trim_end().to_owned())
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

    #[test]
    fn derives_a_short_local_prompt_fallback() {
        assert_eq!(
            prompt_title("Could you please help me fix the parser? Include tests.").as_deref(),
            Some("Fix the parser")
        );
        assert_eq!(
            prompt_title("Let's discuss cars").as_deref(),
            Some("Discuss cars")
        );
        assert_eq!(
            prompt_title("Fix http caching\nInclude tests").as_deref(),
            Some("Fix http caching")
        );
        assert_eq!(
            prompt_title("请修复聊天标题。然后检查测试").as_deref(),
            Some("请修复聊天标题")
        );
        for raw in ["", "   ", "/model", "https://example.com/task/123", "?!"] {
            assert_eq!(prompt_title(raw), None, "{raw}");
        }
        assert_eq!(prompt_title(&"界".repeat(600)).unwrap().chars().count(), 40);
        assert!(
            prompt_title("Fix the terminal title when switching between different projects")
                .unwrap()
                .chars()
                .count()
                <= 40
        );
    }
}
