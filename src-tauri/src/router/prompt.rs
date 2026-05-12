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

/// View of the lead slot the launch prompt needs. `handle` is the
/// slot's in-crew handle (slot_handle); `display_name` and
/// `system_prompt` come from the underlying runner template.
/// Decoupled from `Runner` so the composer doesn't know how the
/// fields are joined.
pub struct LeadView<'a> {
    pub handle: &'a str,
    pub display_name: &'a str,
    pub system_prompt: Option<&'a str>,
}

/// One crewmate in the lead's roster section.
pub struct RosterEntry<'a> {
    pub handle: &'a str,
    pub display_name: &'a str,
    pub lead: bool,
}

/// All inputs for the launch prompt. Borrowed so the caller can compose
/// without copying the runner row.
pub struct LaunchPromptInput<'a> {
    pub lead: LeadView<'a>,
    pub crew_name: &'a str,
    pub mission_goal: &'a str,
    pub roster: &'a [RosterEntry<'a>],
    pub allowed_signals: &'a [SignalType],
}

/// First-user-turn body for a non-lead mission worker. Combines the
/// platform-injected coordination preamble (verbs the worker needs to
/// participate in the bus + reply to the human) with the worker's
/// per-runner system_prompt as a "brief" section. Returns the full
/// composed body, never empty (preamble is always present).
///
/// Delivered as the trailing positional `[PROMPT]` argv at spawn time
/// when the runtime accepts it (see `router::runtime::first_turn_argv`);
/// callers that can't use argv inject the same body via stdin paste.
/// Source of truth lives here so both delivery paths use byte-identical
/// content.
pub fn compose_worker_first_turn(system_prompt: Option<&str>) -> String {
    let user_brief = system_prompt
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let mut out = String::new();
    out.push_str(WORKER_COORDINATION_PREAMBLE);
    if let Some(brief) = user_brief {
        out.push_str("\n\n== Your brief ==\n");
        out.push_str(&brief);
    }
    out
}

/// First-user-turn body for a direct chat. Just the runner's
/// `system_prompt` (persona / role) — no coordination preamble.
/// Direct chats are off-bus, so the worker preamble's verbs
/// (`runner msg post`, `runner status idle`, etc.) don't resolve
/// to anything useful and would mislead the agent.
///
/// Returns None when system_prompt is missing or all-whitespace, so
/// claude-code / codex direct chats boot vanilla in that case.
pub fn compose_direct_first_turn(system_prompt: Option<&str>) -> Option<String> {
    system_prompt
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Platform-injected preamble for non-lead worker spawns. Covers the
/// bus conventions a worker needs to interact with the crew + the
/// human, leaving the user-authored `system_prompt` free to focus on
/// persona / role.
pub(crate) const WORKER_COORDINATION_PREAMBLE: &str = r#"You are a worker in a crew coordinated by the bundled `runner` CLI. The CLI is on your PATH and talks to the rest of the crew + the human operator via a shared event bus. Use these verbs to participate; do not invent your own conventions.

== Coordination ==
- `runner msg read` — read your inbox (pull-based: new messages do NOT auto-print). Run this when you see an `[inbox]` notification or any time you suspect new traffic.
- `runner msg post --to <handle> "<text>"` — direct message to a specific handle. Valid handles: any slot in this crew, plus the reserved virtual handle `human` (the workspace operator).
- `runner msg post "<text>"` — broadcast to the crew (no `--to`).
- `runner signal ask_lead --payload '{"question":"…","context":"…"}'` — escalate to the lead when a load-bearing decision is genuinely ambiguous.
- `runner status idle` — report you've finished the current task. The lead view uses this to dispatch the next slot.

== Replying to the human ==
The human is watching the workspace feed, NOT your TUI. When the human speaks to you directly (raw input lands in your TUI, often prefixed with `[human_said]`), reply via:
    runner msg post --to human "<your reply>"
Plain TUI output (typing into your editor, printing to stdout) stays in your local scrollback only — it never reaches the human. The `--to human` route is the only way your reply lands in the workspace feed."#;

pub fn compose_launch_prompt(input: &LaunchPromptInput<'_>) -> String {
    let mut out = String::new();

    out.push_str(&format!(
        "You are `{}` ({}), lead runner in crew \"{}\".\n\n",
        input.lead.handle, input.lead.display_name, input.crew_name,
    ));

    if let Some(brief) = input.lead.system_prompt {
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
                "- `{}` ({}){}\n",
                r.handle,
                r.display_name,
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
    out.push_str(
        "- Reply to the HUMAN with `runner msg post --to human \"…\"`. The human watches the workspace feed, not your TUI — typing answers into the TUI keeps them in your local scrollback only. `human` is a reserved virtual handle for this two-way path.\n",
    );
    out.push_str("- Read your inbox with `runner msg read` — it's pull-based.\n");
    out.push_str(
        "- Escalate to the human (with structured choices) via `runner signal ask_human --payload '{\"prompt\":\"…\",\"choices\":[\"yes\",\"no\"],\"on_behalf_of\":\"<asker>\"}'`. Plain replies should use `runner msg post --to human` instead.\n",
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

    fn lead<'a>(handle: &'a str, system_prompt: Option<&'a str>) -> LeadView<'a> {
        LeadView {
            handle,
            display_name: "Lead",
            system_prompt,
        }
    }

    #[test]
    fn includes_brief_when_present_and_omits_when_blank() {
        let allowed = [SignalType::new("mission_goal")];
        let prompt = compose_launch_prompt(&LaunchPromptInput {
            lead: lead("lead", Some("Drive coordination.")),
            crew_name: "Alpha",
            mission_goal: "ship v0",
            roster: &[],
            allowed_signals: &allowed,
        });
        assert!(prompt.contains("== Your brief =="));
        assert!(prompt.contains("Drive coordination."));
        assert!(prompt.contains("Goal: ship v0"));
        assert!(prompt.contains("Allowed signal types: mission_goal"));

        let prompt2 = compose_launch_prompt(&LaunchPromptInput {
            lead: lead("lead", None),
            crew_name: "Alpha",
            mission_goal: "ship v0",
            roster: &[],
            allowed_signals: &allowed,
        });
        assert!(!prompt2.contains("== Your brief =="));
    }

    #[test]
    fn empty_goal_renders_placeholder() {
        let prompt = compose_launch_prompt(&LaunchPromptInput {
            lead: lead("lead", None),
            crew_name: "A",
            mission_goal: "",
            roster: &[],
            allowed_signals: &[],
        });
        assert!(prompt.contains("(no goal set"));
    }

    #[test]
    fn roster_section_excludes_self_and_lists_crewmates() {
        let prompt = compose_launch_prompt(&LaunchPromptInput {
            lead: lead("lead", None),
            crew_name: "A",
            mission_goal: "g",
            roster: &[
                RosterEntry {
                    handle: "lead",
                    display_name: "Lead",
                    lead: true,
                },
                RosterEntry {
                    handle: "impl",
                    display_name: "Impl",
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
