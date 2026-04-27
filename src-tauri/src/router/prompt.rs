// Composed launch prompt for the lead, written to stdin on `mission_goal`.
//
// Pure function over the inputs: no I/O, no DB access, no globals — easy to
// unit-test against fixture rosters and goal strings.
//
// The four sections (brief, mission, crewmates, coordination) mirror the
// example in arch §4.3. We diverge from the per-runner spawn-time prompt in
// one place: this is what the *lead* sees on `mission_goal`, not every
// runner's startup prompt. Worker runtime adapters get the runner's own
// `system_prompt` via `--append-system-prompt`-equivalent flags at spawn
// time (see runtime.rs); this composer is the lead's coordination kit.
//
// Output ends with a trailing `\n` so it lands as a single submitted line
// when injected into a TUI's input box.

use runner_core::model::SignalType;

use crate::model::Runner;

/// One crewmate in the lead's roster section.
pub struct RosterEntry<'a> {
    pub handle: &'a str,
    pub display_name: &'a str,
    pub role: &'a str,
    pub lead: bool,
}

/// All inputs for the launch prompt. Borrowed so the caller can compose
/// without copying the runner row.
pub struct LaunchPromptInput<'a> {
    pub lead: &'a Runner,
    pub crew_name: &'a str,
    pub mission_goal: &'a str,
    pub roster: &'a [RosterEntry<'a>],
    pub allowed_signals: &'a [SignalType],
}

pub fn compose_launch_prompt(input: &LaunchPromptInput<'_>) -> String {
    let mut out = String::new();

    out.push_str(&format!(
        "You are `{}` ({}), lead runner in crew \"{}\".\n",
        input.lead.handle, input.lead.display_name, input.crew_name,
    ));
    out.push_str(&format!("Your role: {}.\n\n", input.lead.role));

    if let Some(brief) = input.lead.system_prompt.as_deref() {
        let brief = brief.trim();
        if !brief.is_empty() {
            out.push_str("== Your brief ==\n");
            out.push_str(brief);
            out.push_str("\n\n");
        }
    }

    out.push_str("== Mission ==\n");
    if input.mission_goal.trim().is_empty() {
        out.push_str("Goal: (no goal set; await human_said).\n\n");
    } else {
        out.push_str(&format!("Goal: {}\n\n", input.mission_goal.trim()));
    }

    let crewmates: Vec<&RosterEntry> = input
        .roster
        .iter()
        .filter(|r| r.handle != input.lead.handle)
        .collect();
    if !crewmates.is_empty() {
        out.push_str("== Your crewmates ==\n");
        for r in crewmates {
            out.push_str(&format!(
                "- `{}` ({}, {}){}\n",
                r.handle,
                r.display_name,
                r.role,
                if r.lead { " — lead" } else { "" },
            ));
        }
        out.push('\n');
    }

    out.push_str("== Coordination ==\n");
    out.push_str("- You are the human's counterpart. Workers escalate to you via `ask_lead`.\n");
    out.push_str(
        "- Reply to a worker with `runner msg post --to <handle> \"…\"`; broadcasts omit `--to`.\n",
    );
    out.push_str("- Read your inbox with `runner msg read` — it's pull-based.\n");
    out.push_str(
        "- Escalate to the human with `runner signal ask_human --payload '{\"prompt\":\"…\",\"choices\":[\"yes\",\"no\"],\"on_behalf_of\":\"<asker>\"}'`.\n",
    );
    out.push_str("- Report idle with `runner status idle` so the lead view stays accurate.\n");
    if !input.allowed_signals.is_empty() {
        let names: Vec<&str> = input
            .allowed_signals
            .iter()
            .map(SignalType::as_str)
            .collect();
        out.push_str(&format!("- Allowed signal types: {}.\n", names.join(", ")));
    }

    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use std::collections::HashMap;

    fn runner(handle: &str, system_prompt: Option<&str>) -> Runner {
        Runner {
            id: "r".into(),
            handle: handle.into(),
            display_name: "Lead".into(),
            role: "coordinator".into(),
            runtime: "claude-code".into(),
            command: "/bin/sh".into(),
            args: vec![],
            working_dir: None,
            system_prompt: system_prompt.map(String::from),
            env: HashMap::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn includes_brief_when_present_and_omits_when_blank() {
        let lead = runner("lead", Some("Drive coordination."));
        let allowed = [SignalType::new("mission_goal")];
        let prompt = compose_launch_prompt(&LaunchPromptInput {
            lead: &lead,
            crew_name: "Alpha",
            mission_goal: "ship v0",
            roster: &[],
            allowed_signals: &allowed,
        });
        assert!(prompt.contains("== Your brief =="));
        assert!(prompt.contains("Drive coordination."));
        assert!(prompt.contains("Goal: ship v0"));
        assert!(prompt.contains("Allowed signal types: mission_goal"));

        let lead2 = runner("lead", None);
        let prompt2 = compose_launch_prompt(&LaunchPromptInput {
            lead: &lead2,
            crew_name: "Alpha",
            mission_goal: "ship v0",
            roster: &[],
            allowed_signals: &allowed,
        });
        assert!(!prompt2.contains("== Your brief =="));
    }

    #[test]
    fn empty_goal_renders_placeholder() {
        let lead = runner("lead", None);
        let prompt = compose_launch_prompt(&LaunchPromptInput {
            lead: &lead,
            crew_name: "A",
            mission_goal: "",
            roster: &[],
            allowed_signals: &[],
        });
        assert!(prompt.contains("(no goal set"));
    }

    #[test]
    fn roster_section_excludes_self_and_lists_crewmates() {
        let lead = runner("lead", None);
        let prompt = compose_launch_prompt(&LaunchPromptInput {
            lead: &lead,
            crew_name: "A",
            mission_goal: "g",
            roster: &[
                RosterEntry {
                    handle: "lead",
                    display_name: "Lead",
                    role: "coord",
                    lead: true,
                },
                RosterEntry {
                    handle: "impl",
                    display_name: "Impl",
                    role: "coding",
                    lead: false,
                },
            ],
            allowed_signals: &[],
        });
        assert!(prompt.contains("`impl`"));
        // Self-row must not appear under crewmates.
        let crewmates_section = prompt.split("== Your crewmates ==").nth(1).unwrap();
        assert!(!crewmates_section.contains("`lead`"));
    }
}
