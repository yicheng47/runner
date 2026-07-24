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
    /// Layer-2 team conventions text (`crew.system_prompt_addendum`).
    /// Spliced under a `== Team conventions ==` section between the
    /// "You are X, lead runner in crew Y" intro and the `== Your
    /// brief ==` section. Empty / whitespace-only → no splice. See #54.
    pub crew_addendum: Option<&'a str>,
}

/// First-user-turn body for a non-lead mission worker. Combines the
/// platform-injected coordination preamble (Layer 1 — verbs the
/// worker needs to participate in the bus), the optional crew-level
/// addendum spliced under a `== Team conventions ==` section (Layer
/// 2), and the worker's per-runner system_prompt as a `== Your brief
/// ==` section (Layer 3 — persona). Returns the full composed body,
/// never empty (preamble is always present).
///
/// Delivered as the trailing positional `[PROMPT]` argv at spawn time
/// when the runtime accepts it (see `router::runtime::first_turn_argv`).
pub fn compose_worker_first_turn(
    system_prompt: Option<&str>,
    crew_addendum: Option<&str>,
) -> String {
    let addendum = crew_addendum
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let user_brief = system_prompt
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let mut out = String::new();
    out.push_str(WORKER_COORDINATION_PREAMBLE);
    if let Some(addendum) = addendum {
        out.push_str("\n\n== Team conventions ==\n");
        out.push_str(&addendum);
    }
    if let Some(brief) = user_brief {
        out.push_str("\n\n== Your brief ==\n");
        out.push_str(&brief);
    }
    out
}

/// First-user-turn body for a direct chat. Just the runner's
/// `system_prompt` (persona / role) — no coordination preamble.
/// Direct chats are off-bus, so the worker preamble's verbs
/// (`runner msg post` etc.) don't resolve to anything useful and
/// would mislead the agent.
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
/// bus conventions a worker needs to interact with the crew, leaving
/// the user-authored `system_prompt` free to focus on persona / role.
pub(crate) const WORKER_COORDINATION_PREAMBLE: &str = r#"You are a worker in a crew coordinated by the bundled `runner` CLI. The CLI is on your PATH and talks to the rest of the crew via a shared event bus. Use these verbs to participate; do not invent your own conventions.

== Coordination ==
- `runner msg read` — read your inbox (pull-based: new messages do NOT auto-print). Run this when you see an `[inbox]` notification or any time you suspect new traffic.
- `runner msg post --to <handle> "<text>"` — direct message to a specific handle. Valid handles: any slot in this crew.
- `runner msg post "<text>"` — broadcast to the crew (no `--to`).
- `runner signal ask_lead --payload '{"question":"…","context":"…"}'` — escalate to the lead when a load-bearing decision is genuinely ambiguous.
- Busy/idle is inferred from your terminal activity — no need to call `runner status`."#;

pub fn compose_launch_prompt(input: &LaunchPromptInput<'_>) -> String {
    let mut out = String::new();

    out.push_str(&format!(
        "You are `{}` ({}), lead runner in crew \"{}\".\n\n",
        input.lead.handle, input.lead.display_name, input.crew_name,
    ));

    if let Some(addendum) = input.crew_addendum {
        let addendum = addendum.trim();
        if !addendum.is_empty() {
            out.push_str("== Team conventions ==\n");
            out.push_str(addendum);
            out.push_str("\n\n");
        }
    }

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
        out.push_str(
            "Goal: (no goal set; await the operator's instructions in your terminal).\n\n",
        );
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
    out.push_str("- The operator watches the terminals and types directly into a runner's pane.\n");
    out.push_str("- Read your inbox with `runner msg read` — it's pull-based.\n");
    out.push_str(
        "- Escalate to the human (with structured choices) via `runner signal ask_human --payload '{\"prompt\":\"…\",\"choices\":[\"yes\",\"no\"],\"on_behalf_of\":\"<asker>\"}'`.\n",
    );
    out.push_str(
        "- Busy/idle is inferred from your terminal activity — no need to call `runner status`.\n",
    );
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
            crew_addendum: None,
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
            crew_addendum: None,
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
            crew_addendum: None,
        });
        assert!(prompt
            .contains("Goal: (no goal set; await the operator's instructions in your terminal)."));
    }

    #[test]
    fn composed_prompts_omit_reply_to_human_protocol() {
        let worker = compose_worker_first_turn(None, None);
        let lead = compose_launch_prompt(&LaunchPromptInput {
            lead: lead("lead", None),
            crew_name: "A",
            mission_goal: "g",
            roster: &[],
            allowed_signals: &[],
            crew_addendum: None,
        });

        for prompt in [&worker, &lead] {
            assert!(!prompt.contains("--to human"), "got: {prompt}");
            assert!(!prompt.contains("`human`"), "got: {prompt}");
            assert!(!prompt.contains("[human_said]"), "got: {prompt}");
        }
        assert!(!worker.contains("human"), "got: {worker}");
        assert!(lead.contains(
            "The operator watches the terminals and types directly into a runner's pane."
        ));
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
            crew_addendum: None,
        });
        assert!(prompt.contains("`impl`"));
        // Self-row must not appear under crewmates.
        let crewmates_section = prompt.split("== Your crewmates ==").nth(1).unwrap();
        assert!(!crewmates_section.contains("`lead`"));
    }

    #[test]
    fn worker_first_turn_with_none_addendum_matches_no_addendum_baseline() {
        // Regression guard for #54: existing mission spawns (no crew
        // addendum set) must produce byte-identical output to the
        // pre-#54 composer, so seeded Build squad rows / any crew that
        // leaves the addendum NULL keep the exact prompt they had.
        let with_brief = compose_worker_first_turn(Some("WORKER_BRIEF"), None);
        let without_brief = compose_worker_first_turn(None, None);

        assert!(with_brief.starts_with(WORKER_COORDINATION_PREAMBLE));
        assert!(with_brief.contains("== Your brief =="));
        assert!(with_brief.contains("WORKER_BRIEF"));

        // No addendum → preamble is the entire body when brief is None.
        assert_eq!(without_brief, WORKER_COORDINATION_PREAMBLE);
    }

    #[test]
    fn worker_first_turn_splices_addendum_between_preamble_and_brief() {
        let body = compose_worker_first_turn(Some("WORKER_BRIEF"), Some("TEAM_TEXT"));
        assert!(body.contains(WORKER_COORDINATION_PREAMBLE));
        assert!(
            body.contains("== Team conventions =="),
            "addendum must be wrapped in a `== Team conventions ==` section; got: {body}",
        );
        assert!(body.contains("TEAM_TEXT"));
        assert!(body.contains("== Your brief =="));
        let preamble_pos = body.find("Coordination ==").unwrap();
        let header_pos = body.find("== Team conventions ==").unwrap();
        let addendum_pos = body.find("TEAM_TEXT").unwrap();
        let brief_pos = body.find("== Your brief ==").unwrap();
        assert!(
            preamble_pos < header_pos,
            "preamble must come before the team-conventions header; got body: {body}",
        );
        assert!(
            header_pos < addendum_pos,
            "team-conventions header must come before the addendum text; got body: {body}",
        );
        assert!(
            addendum_pos < brief_pos,
            "addendum must come before brief; got body: {body}",
        );
    }

    #[test]
    fn worker_first_turn_whitespace_only_addendum_collapses_to_none() {
        let with_blanks = compose_worker_first_turn(Some("BRIEF"), Some("   \n\t  "));
        let baseline = compose_worker_first_turn(Some("BRIEF"), None);
        assert_eq!(
            with_blanks, baseline,
            "whitespace-only addendum must be treated as None",
        );
    }

    #[test]
    fn launch_prompt_splices_addendum_between_intro_and_brief() {
        let allowed = [SignalType::new("mission_goal")];
        let with_addendum = compose_launch_prompt(&LaunchPromptInput {
            lead: lead("lead", Some("LEAD_BRIEF")),
            crew_name: "Alpha",
            mission_goal: "ship v0",
            roster: &[],
            allowed_signals: &allowed,
            crew_addendum: Some("TEAM_TEXT"),
        });
        assert!(
            with_addendum.contains("== Team conventions =="),
            "addendum must be wrapped in a `== Team conventions ==` section; got: {with_addendum}",
        );
        let intro_pos = with_addendum.find("lead runner in crew").unwrap();
        let header_pos = with_addendum.find("== Team conventions ==").unwrap();
        let addendum_pos = with_addendum.find("TEAM_TEXT").unwrap();
        let brief_pos = with_addendum.find("== Your brief ==").unwrap();
        assert!(intro_pos < header_pos);
        assert!(header_pos < addendum_pos);
        assert!(addendum_pos < brief_pos);

        // None addendum is byte-identical to the no-addendum baseline.
        let baseline = compose_launch_prompt(&LaunchPromptInput {
            lead: lead("lead", Some("LEAD_BRIEF")),
            crew_name: "Alpha",
            mission_goal: "ship v0",
            roster: &[],
            allowed_signals: &allowed,
            crew_addendum: None,
        });
        let whitespace = compose_launch_prompt(&LaunchPromptInput {
            lead: lead("lead", Some("LEAD_BRIEF")),
            crew_name: "Alpha",
            mission_goal: "ship v0",
            roster: &[],
            allowed_signals: &allowed,
            crew_addendum: Some("   \n  "),
        });
        assert_eq!(whitespace, baseline);
        assert!(!baseline.contains("TEAM_TEXT"));
    }
}
