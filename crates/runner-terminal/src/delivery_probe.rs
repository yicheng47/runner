//! Confirmation tracks the rows changed by typing, never the terminal cursor.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProbeState {
    Present,
    Cleared,
    Unknown,
}

#[derive(Clone, Debug)]
pub struct InputProbe {
    before: Vec<String>,
    text: String,
    rows: Vec<(usize, String, String)>,
}

impl InputProbe {
    pub fn new(before: Vec<String>, text: &str) -> Self {
        Self {
            before,
            text: text.lines().last().unwrap_or_default().to_owned(),
            rows: Vec::new(),
        }
    }

    /// Capture before Enter, so a transcript echo after submission cannot become the input.
    pub fn locate(&mut self, after: &[String]) {
        self.rows.clear();
        if self.text.is_empty() || self.before.len() != after.len() {
            return;
        }
        for start in 0..after.len() {
            if self.before[start] == after[start] {
                continue;
            }
            let mut joined = String::new();
            for end in start..after.len() {
                joined.push_str(&after[end]);
                if let Some(offset) = joined.find(&self.text) {
                    if joined.matches(&self.text).count() != 1 {
                        self.rows.clear();
                        return;
                    }
                    let mut candidate = Vec::new();
                    let mut position = 0;
                    for (row, full) in after.iter().enumerate().take(end + 1).skip(start) {
                        let cells = full.as_str();
                        let first = offset.saturating_sub(position).min(cells.len());
                        let last = (offset + self.text.len())
                            .saturating_sub(position)
                            .min(cells.len());
                        if first < last {
                            candidate.push((row, full.clone(), cells[first..last].to_owned()));
                        }
                        position += cells.len();
                    }
                    if candidate
                        .iter()
                        .any(|(row, full, _)| self.before[*row] != *full)
                    {
                        if !self.rows.is_empty() && self.rows != candidate {
                            self.rows.clear();
                            return;
                        }
                        self.rows = candidate;
                    }
                    break;
                }
                if joined.len() > self.text.len() + after[start].len() {
                    break;
                }
            }
        }
    }

    pub fn state(&self, current: &[String]) -> ProbeState {
        if self.rows.is_empty() || current.len() != self.before.len() {
            return ProbeState::Unknown;
        }
        if self
            .rows
            .iter()
            .all(|(row, full, _)| current.get(*row) == Some(full))
        {
            return ProbeState::Present;
        }
        if self
            .rows
            .iter()
            .all(|(row, _, typed)| current.get(*row).is_some_and(|text| !text.contains(typed)))
        {
            ProbeState::Cleared
        } else {
            ProbeState::Unknown
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn rows(rows: &[&str]) -> Vec<String> {
        rows.iter().map(|row| (*row).to_owned()).collect()
    }

    #[test]
    fn transcript_echo_above_the_input_does_not_prevent_confirmation() {
        for prefix in ["❯ ", "› ", "> ", "│ ", "  "] {
            let before = rows(&["old output", prefix, "footer"]);
            let typed = rows(&["old output", &format!("{prefix}[inbox] task"), "footer"]);
            let submitted = rows(&["[inbox] task", prefix, "footer"]);
            let mut probe = InputProbe::new(before, "[inbox] task");
            probe.locate(&typed);
            assert_eq!(probe.state(&typed), ProbeState::Present);
            assert_eq!(probe.state(&submitted), ProbeState::Cleared);
        }
    }
    #[test]
    fn only_exact_same_rows_allow_another_enter() {
        let mut probe = InputProbe::new(rows(&["output", "› ", ""]), "[inbox] task");
        probe.locate(&rows(&["output", "› [inbox] task", ""]));
        assert_eq!(
            probe.state(&rows(&["output", "❯ [inbox] task", ""])),
            ProbeState::Unknown
        );
        assert_eq!(
            probe.state(&rows(&["› [inbox] task", "new output", ""])),
            ProbeState::Cleared
        );
        assert_eq!(probe.state(&rows(&["› [inbox] task"])), ProbeState::Unknown);
    }
    #[test]
    fn wrapped_notifications_preserve_boundary_spaces_and_mid_word_cells() {
        let text = "[inbox] 2 new messages — run `runner msg read` to view.";
        for split in ["[inbox] 2 new ".len(), "[inbox] 2 new mess".len()] {
            let mut probe = InputProbe::new(rows(&["out", "› ", "", "footer"]), text);
            let typed = vec![
                "out".into(),
                format!("› {}", &text[..split]),
                text[split..].into(),
                "footer".into(),
            ];
            probe.locate(&typed);
            assert_eq!(probe.state(&typed), ProbeState::Present);
            assert_eq!(
                probe.state(&rows(&["out", "› ", "", "footer"])),
                ProbeState::Cleared
            );
        }
    }
    #[test]
    fn an_unfound_or_preexisting_text_is_unknown() {
        let before = rows(&["[inbox] task", "› "]);
        let mut probe = InputProbe::new(before.clone(), "[inbox] task");
        probe.locate(&before);
        assert_eq!(probe.state(&before), ProbeState::Unknown);
    }
}

#[cfg(test)]
mod ambiguity_tests {
    use super::*;
    #[test]
    fn simultaneous_transcript_and_composer_matches_are_unknown() {
        let mut probe = InputProbe::new(vec!["old output".into(), "› ".into()], "[inbox] task");
        let rows = vec!["[inbox] task".into(), "› [inbox] task".into()];
        probe.locate(&rows);
        assert_eq!(probe.state(&rows), ProbeState::Unknown);
    }
}

#[cfg(test)]
mod wrapped_grid_tests {
    use super::*;
    use crate::replay::{feed, new_term, row_cells, Terminal};

    #[test]
    fn actual_terminal_cells_keep_a_space_at_the_notification_wrap_boundary() {
        let text = "[inbox] 2 new messages — run `runner msg read` to view.";
        let boundary = ("> ".len() + "[inbox] 2 new ".len()) as u16;
        for cols in [boundary, boundary + 4] {
            let mut term = new_term(cols, 8);
            feed(&mut term, b"> ");
            let snapshot = |term: &Terminal| {
                (0..term.rows())
                    .map(|row| row_cells(term, row as i32))
                    .collect::<Vec<_>>()
            };
            let mut probe = InputProbe::new(snapshot(&term), text);
            feed(&mut term, text.as_bytes());
            probe.locate(&snapshot(&term));
            assert_eq!(probe.state(&snapshot(&term)), ProbeState::Present);
            if cols == boundary {
                assert!(row_cells(&term, 0).ends_with(' '));
            }
            feed(&mut term, b"\x1b[2J\x1b[H> ");
            assert_eq!(probe.state(&snapshot(&term)), ProbeState::Cleared);
        }
    }
}
