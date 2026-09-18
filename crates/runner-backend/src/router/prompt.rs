// Composed session prompts, split across each runtime's prompt channels.
//
// Pure function over the inputs: no I/O, no DB access, no globals — easy to
// unit-test against fixture rosters and goal strings.
//
// The four sections (brief, mission, crewmates, coordination) mirror the
// example in arch §6. The runtime adapter sends the composed body at spawn;
// pi splits the mission section into the lead's only first turn and writes
// the other sections to its per-session system-prompt file.

use runner_core::model::SignalType;

use crate::model::Runtime;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionPromptKind {
    Direct,
    Worker,
    Lead,
}

/// Split an already-composed session prompt across the runtime's native
/// prompt channels. The four established runtimes keep the complete body as
/// their first user turn. pi receives the persona and coordination layers as
/// a system prompt; only a lead's mission section remains a first turn.
pub fn split_session_prompt(
    runtime: Option<Runtime>,
    kind: SessionPromptKind,
    composed: Option<String>,
) -> (Option<String>, Option<String>) {
    if runtime != Some(Runtime::Pi) {
        return (None, composed);
    }
    let Some(composed) = composed else {
        return (None, None);
    };
    match kind {
        SessionPromptKind::Direct | SessionPromptKind::Worker => (Some(composed), None),
        SessionPromptKind::Lead => {
            unreachable!("pi lead prompts must be composed from LaunchPromptInput")
        }
    }
}

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
    /// "You are X, the lead of crew Y" intro and the `== Your
    /// brief ==` section. Empty / whitespace-only → no splice. See #54.
    pub crew_addendum: Option<&'a str>,
}

/// First-user-turn body for a non-lead mission worker. Combines the
/// platform-injected coordination preamble (Layer 1 — verbs the
/// worker needs to participate in the bus), the optional crew-level
/// addendum spliced under a `== Team conventions ==` section (Layer
/// 2), and the worker's per-role system_prompt as a `== Your brief
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

struct LaunchPromptSections {
    before_mission: String,
    mission: String,
    after_mission: String,
}

fn compose_launch_prompt_sections(input: &LaunchPromptInput<'_>) -> LaunchPromptSections {
    let mut before_mission = String::new();

    before_mission.push_str(&format!(
        "You are `{}` ({}), the lead of crew \"{}\".\n\n",
        input.lead.handle, input.lead.display_name, input.crew_name,
    ));

    if let Some(addendum) = input.crew_addendum {
        let addendum = addendum.trim();
        if !addendum.is_empty() {
            before_mission.push_str("== Team conventions ==\n");
            before_mission.push_str(addendum);
            before_mission.push_str("\n\n");
        }
    }

    if let Some(brief) = input.lead.system_prompt {
        let brief = brief.trim();
        if !brief.is_empty() {
            before_mission.push_str("== Your brief ==\n");
            before_mission.push_str(brief);
            before_mission.push_str("\n\n");
        }
    }

    let mut mission = String::from("== Mission ==\n");
    if input.mission_goal.trim().is_empty() {
        mission.push_str(
            "Goal: (no goal set; await the operator's instructions in your terminal).\n\n",
        );
    } else {
        mission.push_str(&format!("Goal: {}\n\n", input.mission_goal.trim()));
    }

    let mut after_mission = String::new();
    let crewmates: Vec<&RosterEntry> = input
        .roster
        .iter()
        .filter(|r| r.handle != input.lead.handle)
        .collect();
    if !crewmates.is_empty() {
        after_mission.push_str("== Your crewmates ==\n");
        for r in crewmates {
            after_mission.push_str(&format!(
                "- `{}` ({}){}\n",
                r.handle,
                r.display_name,
                if r.lead { " — lead" } else { "" },
            ));
        }
        after_mission.push('\n');
    }

    after_mission.push_str("== Coordination ==\n");
    after_mission
        .push_str("- You are the human's counterpart. Workers escalate to you via `ask_lead`.\n");
    after_mission.push_str(
        "- Reply to a worker with `runner msg post --to <handle> \"…\"`; broadcasts omit `--to`.\n",
    );
    after_mission.push_str(
        "- The operator watches the terminals and types directly into a runner's pane.\n",
    );
    after_mission.push_str("- Read your inbox with `runner msg read` — it's pull-based.\n");
    after_mission.push_str(
        "- Escalate to the human (with structured choices) via `runner signal ask_human --payload '{\"prompt\":\"…\",\"choices\":[\"yes\",\"no\"],\"on_behalf_of\":\"<asker>\"}'`.\n",
    );
    after_mission.push_str(
        "- Busy/idle is inferred from your terminal activity — no need to call `runner status`.\n",
    );
    if !input.allowed_signals.is_empty() {
        let names: Vec<&str> = input
            .allowed_signals
            .iter()
            .map(SignalType::as_str)
            .collect();
        after_mission.push_str(&format!("- Allowed signal types: {}.\n", names.join(", ")));
    }

    after_mission.push('\n');
    LaunchPromptSections {
        before_mission,
        mission,
        after_mission,
    }
}

pub fn compose_launch_prompt(input: &LaunchPromptInput<'_>) -> String {
    let sections = compose_launch_prompt_sections(input);
    let mut out = sections.before_mission;
    out.push_str(&sections.mission);
    out.push_str(&sections.after_mission);
    out
}

pub fn compose_lead_prompt_channels(
    runtime: Option<Runtime>,
    input: &LaunchPromptInput<'_>,
) -> (Option<String>, Option<String>) {
    if runtime != Some(Runtime::Pi) {
        return (None, Some(compose_launch_prompt(input)));
    }
    let sections = compose_launch_prompt_sections(input);
    let mut system_prompt = sections.before_mission;
    system_prompt.push_str(&sections.after_mission);
    (Some(system_prompt), Some(sections.mission))
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

    fn launch_fixture() -> String {
        let roster = [
            RosterEntry {
                handle: "lead",
                display_name: "Lead",
                lead: true,
            },
            RosterEntry {
                handle: "worker",
                display_name: "Worker",
                lead: false,
            },
        ];
        let signals = [SignalType::new("mission_goal")];
        compose_launch_prompt(&LaunchPromptInput {
            lead: lead("lead", Some("LEAD_BRIEF")),
            crew_name: "Alpha",
            mission_goal: "@ship - safely",
            roster: &roster,
            allowed_signals: &signals,
            crew_addendum: Some("TEAM_TEXT"),
        })
    }

    #[test]
    fn established_runtimes_keep_composed_bodies_byte_identical() {
        let direct = compose_direct_first_turn(Some("  persona  "));
        let worker = Some(compose_worker_first_turn(Some("brief"), Some("team")));
        let lead = Some(launch_fixture());
        for runtime in [
            Runtime::ClaudeCode,
            Runtime::Codex,
            Runtime::Trae,
            Runtime::Copilot,
        ] {
            for (kind, body) in [
                (SessionPromptKind::Direct, direct.clone()),
                (SessionPromptKind::Worker, worker.clone()),
                (SessionPromptKind::Lead, lead.clone()),
            ] {
                assert_eq!(
                    split_session_prompt(Some(runtime), kind, body.clone()),
                    (None, body)
                );
            }
        }
    }

    #[test]
    fn compose_launch_prompt_matches_established_literal_contract() {
        let roster = [
            RosterEntry {
                handle: "coder",
                display_name: "Coder",
                lead: true,
            },
            RosterEntry {
                handle: "reviewer",
                display_name: "Reviewer",
                lead: false,
            },
            RosterEntry {
                handle: "tester",
                display_name: "Tester",
                lead: false,
            },
        ];
        let signals = [
            SignalType::new("mission_goal"),
            SignalType::new("ask_human"),
        ];
        let prompt = compose_launch_prompt(&LaunchPromptInput {
            lead: LeadView {
                handle: "coder",
                display_name: "Coder",
                system_prompt: Some("Ship code."),
            },
            crew_name: "Codex peer",
            mission_goal: "Implement feature",
            roster: &roster,
            allowed_signals: &signals,
            crew_addendum: Some("Review first."),
        });

        assert_eq!(
            prompt,
            r#"You are `coder` (Coder), the lead of crew "Codex peer".

== Team conventions ==
Review first.

== Your brief ==
Ship code.

== Mission ==
Goal: Implement feature

== Your crewmates ==
- `reviewer` (Reviewer)
- `tester` (Tester)

== Coordination ==
- You are the human's counterpart. Workers escalate to you via `ask_lead`.
- Reply to a worker with `runner msg post --to <handle> "…"`; broadcasts omit `--to`.
- The operator watches the terminals and types directly into a runner's pane.
- Read your inbox with `runner msg read` — it's pull-based.
- Escalate to the human (with structured choices) via `runner signal ask_human --payload '{"prompt":"…","choices":["yes","no"],"on_behalf_of":"<asker>"}'`.
- Busy/idle is inferred from your terminal activity — no need to call `runner status`.
- Allowed signal types: mission_goal, ask_human.

"#
        );
    }

    #[test]
    fn pi_uses_system_prompt_for_direct_and_worker_sessions() {
        for (kind, body) in [
            (SessionPromptKind::Direct, "persona"),
            (SessionPromptKind::Worker, "coordination and brief"),
        ] {
            assert_eq!(
                split_session_prompt(Some(Runtime::Pi), kind, Some(body.into())),
                (Some(body.into()), None),
            );
        }
    }

    #[test]
    fn pi_lead_keeps_only_the_mission_as_first_turn() {
        let roster = [
            RosterEntry {
                handle: "lead",
                display_name: "Lead",
                lead: true,
            },
            RosterEntry {
                handle: "worker",
                display_name: "Worker",
                lead: false,
            },
        ];
        let signals = [SignalType::new("mission_goal")];
        let input = LaunchPromptInput {
            lead: lead("lead", Some("LEAD_BRIEF\n== Mission ==\nstill the brief")),
            crew_name: "Alpha",
            mission_goal: "@ship - safely\n== Coordination ==\nstill the goal",
            roster: &roster,
            allowed_signals: &signals,
            crew_addendum: Some("TEAM_TEXT"),
        };
        let composed = compose_launch_prompt(&input);
        let (system_prompt, first_turn) = compose_lead_prompt_channels(Some(Runtime::Pi), &input);
        let system_prompt = system_prompt.unwrap();
        let first_turn = first_turn.unwrap();
        assert_eq!(
            first_turn,
            "== Mission ==\nGoal: @ship - safely\n== Coordination ==\nstill the goal\n\n"
        );
        assert!(system_prompt.contains("LEAD_BRIEF\n== Mission ==\nstill the brief"));
        assert!(system_prompt.contains("TEAM_TEXT"));
        assert!(system_prompt.contains("== Your crewmates =="));
        assert!(system_prompt.contains("== Coordination =="));
        let insertion = system_prompt.find("== Your crewmates ==").unwrap();
        let mut recomposed = system_prompt;
        recomposed.insert_str(insertion, &first_turn);
        assert_eq!(composed, recomposed);
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
        let intro_pos = with_addendum.find("the lead of crew").unwrap();
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
