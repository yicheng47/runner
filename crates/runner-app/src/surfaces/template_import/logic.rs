use std::collections::{BTreeMap, HashMap};

use gpui::prelude::*;
use gpui::{rems, svg};
use runner_app::ui::{SelectLeading, SelectOption};
use runner_core::protocol::model::{CodexSpeed, Runtime};
use runner_core::template::{
    CrewTemplate, RoleTemplate, SlotTemplate, Template, TemplatePreviewPlan, MAX_TEMPLATE_BYTES,
};

use crate::chat_icon::ChatIcon;
use crate::surfaces::roles::logic::runtime_display_name;
use crate::theme;

pub(crate) const SAMPLE_CREWS_URL: &str = "https://runnersh.dev/crews";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SourceSegment {
    Text(String),
    Marker(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ReaderItemKind {
    Role {
        handle: String,
        display_name: String,
        runtime: Runtime,
        model: Option<String>,
        effort: Option<String>,
        codex_speed: Option<CodexSpeed>,
    },
    Conventions,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ReaderItem {
    pub(crate) kind: ReaderItemKind,
    pub(crate) label: String,
    pub(crate) text: String,
    pub(crate) lines: usize,
    pub(crate) bytes: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BulkMissingRuntime {
    pub(crate) runtime: Runtime,
    pub(crate) slot_indices: Vec<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct EffectiveSlotSummary {
    pub(crate) template_runtime: Runtime,
    pub(crate) effective_runtime: Runtime,
    pub(crate) template_installed: bool,
    pub(crate) effective_installed: bool,
    pub(crate) switched: bool,
    pub(crate) model: Option<String>,
    pub(crate) effort: Option<String>,
    pub(crate) codex_speed: Option<CodexSpeed>,
    pub(crate) has_slot_override: bool,
}

pub(crate) fn format_prompt_size(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    }
}

pub(crate) fn prompt_excerpt(text: &str, max_lines: usize) -> String {
    let lines: Vec<&str> = text.lines().take(max_lines).collect();
    let mut out = lines.join("\n");
    if text.lines().count() > max_lines {
        out.push_str("\n…");
    }
    out
}

/// Matches C0/C1 control characters (except `\t`), Unicode `Cf` format
/// characters (including bidi controls, zero-width joiners/spaces, Arabic/Syriac
/// format marks, invisible math operators, interlinear annotations, and the
/// Unicode Tag block `U+E0000..=U+E007F`), variation selectors, Hangul/Braille
/// blank fillers, and line/paragraph separators so hidden prompt text is always
/// surfaced in Source view.
pub(crate) fn is_visible_control_or_invisible(ch: char) -> bool {
    if ch == '\t' {
        return false;
    }
    if ch.is_control() {
        return true;
    }
    matches!(
        ch as u32,
        0x00AD
            | 0x034F
            | 0x0600..=0x0605
            | 0x061C
            | 0x06DD
            | 0x070F
            | 0x0890..=0x0891
            | 0x08E2
            | 0x115F..=0x1160
            | 0x17B4..=0x17B5
            | 0x180B..=0x180F
            | 0x200B..=0x200F
            | 0x2028..=0x202E
            | 0x2060..=0x206F
            | 0x2800
            | 0x3164
            | 0xFE00..=0xFE0F
            | 0xFEFF
            | 0xFFA0
            | 0xFFF9..=0xFFFB
            | 0x110BD
            | 0x110CD
            | 0x13430..=0x1343F
            | 0x1BCA0..=0x1BCA3
            | 0x1D173..=0x1D17A
            | 0xE0000..=0xE007F
            | 0xE0100..=0xE01EF
    )
}

/// Splits a source prompt line into visible text runs and explicit `[U+XXXX]`
/// markers for invisible, bidirectional override, and control characters so
/// hidden prompt instructions can never pass unnoticed during review.
pub(crate) fn source_line_segments(line: &str) -> Vec<SourceSegment> {
    let mut segments = Vec::new();
    let mut buffer = String::new();
    for ch in line.chars() {
        if is_visible_control_or_invisible(ch) {
            if !buffer.is_empty() {
                segments.push(SourceSegment::Text(std::mem::take(&mut buffer)));
            }
            segments.push(SourceSegment::Marker(format!("[U+{:04X}]", ch as u32)));
        } else if ch == '\t' {
            buffer.push_str("    ");
        } else {
            buffer.push(ch);
        }
    }
    if !buffer.is_empty() {
        segments.push(SourceSegment::Text(buffer));
    }
    segments
}

pub(crate) fn validate_template_bytes(bytes: &[u8]) -> Result<String, crate::i18n::Text> {
    if bytes.len() > MAX_TEMPLATE_BYTES {
        return Err(crate::i18n::message("template.file_too_large"));
    }
    std::str::from_utf8(bytes)
        .map(str::to_owned)
        .map_err(|_| crate::i18n::message("template.file_not_utf8"))
}

pub(crate) fn runtime_icon_leading(runtime: Runtime) -> SelectLeading {
    let key = runtime.key();
    let icon = ChatIcon::for_runtime(key);
    SelectLeading::new(format!("template-rt-{key}"), move |size| {
        icon.render(rems(size / 16.), icon.color(theme::muted(), true), true)
    })
}

pub(crate) fn keep_uninstalled_leading() -> SelectLeading {
    SelectLeading::new("template-rt-keep-warn", move |size| {
        svg()
            .flex_none()
            .path("triangle-alert.svg")
            .size(rems(size / 16.))
            .text_color(theme::warning())
            .into_any_element()
    })
}

/// Builds dropdown options for replacing an uninstalled template runtime:
/// installed built-in runtimes first under "Installed on this machine", then
/// a divider and "Keep <Runtime> (Not installed)".
pub(crate) fn missing_runtime_options(
    installed: &[Runtime],
    original: Runtime,
) -> Vec<SelectOption> {
    let mut options = Vec::new();
    for (index, &rt) in installed.iter().enumerate() {
        let mut opt = SelectOption::new(rt.key(), runtime_display_name(rt.key()))
            .leading(runtime_icon_leading(rt));
        if index == 0 {
            opt = opt.section(crate::i18n::message("template.installed_on_this_machine"));
        }
        options.push(opt);
    }
    let keep = SelectOption::new(
        original.key(),
        crate::i18n::messagef(
            "template.keep_not_installed",
            &[("runtime", runtime_display_name(original.key()))],
        ),
    )
    .leading(keep_uninstalled_leading())
    .separator_before(!options.is_empty());
    options.push(keep);
    options
}

/// Builds options for the bulk-switch banner dropdown (only installed runtimes).
pub(crate) fn bulk_runtime_options(installed: &[Runtime]) -> Vec<SelectOption> {
    installed
        .iter()
        .enumerate()
        .map(|(index, &rt)| {
            let mut opt = SelectOption::new(rt.key(), runtime_display_name(rt.key()))
                .leading(runtime_icon_leading(rt));
            if index == 0 {
                opt = opt.section(crate::i18n::message("template.installed_on_this_machine"));
            }
            opt
        })
        .collect()
}

pub(crate) fn effective_slot_summary(
    slot: &SlotTemplate,
    role: &RoleTemplate,
    override_runtime: Option<Runtime>,
    installed: &[Runtime],
) -> EffectiveSlotSummary {
    let template_runtime = slot.runtime.unwrap_or(role.runtime);
    let effective_runtime = override_runtime.unwrap_or(template_runtime);
    let template_installed = installed.contains(&template_runtime);
    let effective_installed = installed.contains(&effective_runtime);
    let switched = effective_runtime != template_runtime;
    let has_slot_override = slot.runtime.is_some()
        || slot.model.is_some()
        || slot.effort.is_some()
        || slot.codex_speed.is_some();
    let (model, effort, codex_speed) = if switched {
        (None, None, None)
    } else if slot.runtime.is_some() && slot.runtime != Some(role.runtime) {
        (slot.model.clone(), slot.effort.clone(), slot.codex_speed)
    } else {
        (
            slot.model.clone().or_else(|| role.model.clone()),
            slot.effort.clone().or_else(|| role.effort.clone()),
            slot.codex_speed.or(role.codex_speed),
        )
    };
    EffectiveSlotSummary {
        template_runtime,
        effective_runtime,
        template_installed,
        effective_installed,
        switched,
        model,
        effort,
        codex_speed,
        has_slot_override,
    }
}

/// Finds every uninstalled runtime currently shared by 2 or more slots in a crew
/// template so the preview can display a one-click bulk switch banner.
pub(crate) fn bulk_missing_runtimes(
    crew: &CrewTemplate,
    slot_overrides: &BTreeMap<String, Runtime>,
    installed: &[Runtime],
) -> Vec<BulkMissingRuntime> {
    let roles_by_handle: HashMap<&str, &RoleTemplate> = crew
        .roles
        .iter()
        .map(|role| (role.handle.as_str(), role))
        .collect();
    let mut grouped: BTreeMap<String, (Runtime, Vec<usize>)> = BTreeMap::new();
    for (index, slot) in crew.slots.iter().enumerate() {
        let Some(role) = roles_by_handle.get(slot.role.as_str()) else {
            continue;
        };
        let template_rt = slot.runtime.unwrap_or(role.runtime);
        let effective_rt = slot_overrides
            .get(&slot.handle)
            .copied()
            .unwrap_or(template_rt);
        if !installed.contains(&effective_rt) {
            grouped
                .entry(effective_rt.key().to_owned())
                .or_insert_with(|| (effective_rt, Vec::new()))
                .1
                .push(index);
        }
    }
    grouped
        .into_values()
        .filter(|(_, indices)| indices.len() >= 2)
        .map(|(runtime, slot_indices)| BulkMissingRuntime {
            runtime,
            slot_indices,
        })
        .collect()
}

pub(crate) fn build_reader_items(
    preview: &TemplatePreviewPlan,
    runtime_overrides: &BTreeMap<String, Runtime>,
) -> Vec<ReaderItem> {
    let mut items = Vec::new();
    match &preview.template {
        Template::Crew(crew) => {
            let roles_by_handle: HashMap<&str, &RoleTemplate> = crew
                .roles
                .iter()
                .map(|role| (role.handle.as_str(), role))
                .collect();
            for slot in &crew.slots {
                let Some(role) = roles_by_handle.get(slot.role.as_str()) else {
                    continue;
                };
                let summary = effective_slot_summary(
                    slot,
                    role,
                    runtime_overrides.get(&slot.handle).copied(),
                    &preview.installed_runtimes,
                );
                let text = role.system_prompt.clone().unwrap_or_default();
                let (bytes, lines) = runner_core::template::prompt_stats(Some(&text));
                items.push(ReaderItem {
                    kind: ReaderItemKind::Role {
                        handle: slot.handle.clone(),
                        display_name: role.display_name.clone(),
                        runtime: summary.effective_runtime,
                        model: summary.model,
                        effort: summary.effort,
                        codex_speed: summary.codex_speed,
                    },
                    label: format!("@{}", slot.handle),
                    text,
                    lines,
                    bytes,
                });
            }
            if let Some(conventions) = crew.crew.conventions.as_deref().filter(|s| !s.is_empty()) {
                let (bytes, lines) = runner_core::template::prompt_stats(Some(conventions));
                items.push(ReaderItem {
                    kind: ReaderItemKind::Conventions,
                    label: crate::i18n::text("template.team_conventions").to_owned(),
                    text: conventions.to_owned(),
                    lines,
                    bytes,
                });
            }
        }
        Template::Role(standalone) => {
            let role = &standalone.role;
            let override_rt = runtime_overrides.get(&role.handle).copied();
            let effective_runtime = override_rt.unwrap_or(role.runtime);
            let switched = effective_runtime != role.runtime;
            let (model, effort, codex_speed) = if switched {
                (None, None, None)
            } else {
                (role.model.clone(), role.effort.clone(), role.codex_speed)
            };
            let text = role.system_prompt.clone().unwrap_or_default();
            let (bytes, lines) = runner_core::template::prompt_stats(Some(&text));
            items.push(ReaderItem {
                kind: ReaderItemKind::Role {
                    handle: role.handle.clone(),
                    display_name: role.display_name.clone(),
                    runtime: effective_runtime,
                    model,
                    effort,
                    codex_speed,
                },
                label: format!("@{}", role.handle),
                text,
                lines,
                bytes,
            });
        }
    }
    items
}

pub(crate) fn extract_error_line(message: &str) -> Option<usize> {
    for prefix in ["(line ", "line "] {
        if let Some(idx) = message.find(prefix) {
            let rest = &message[idx + prefix.len()..];
            let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(line) = digits.parse::<usize>() {
                return Some(line);
            }
        }
    }
    None
}

pub(crate) fn runtime_meta_line(
    runtime: Runtime,
    model: Option<&str>,
    effort: Option<&str>,
    codex_speed: Option<CodexSpeed>,
) -> String {
    let mut parts = vec![runtime_display_name(runtime.key())];
    if let Some(model) = model.filter(|s| !s.trim().is_empty()) {
        parts.push(model.to_owned());
    }
    if let Some(effort) = effort.filter(|s| !s.trim().is_empty()) {
        parts.push(effort.to_owned());
    }
    if runtime == Runtime::Codex {
        if let Some(speed) = codex_speed {
            parts.push(match speed {
                CodexSpeed::Standard => crate::i18n::text("ui.standard").to_owned(),
                CodexSpeed::Fast => crate::i18n::text("ui.fast").to_owned(),
            });
        }
    }
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use runner_core::template::parse_template;

    #[test]
    fn source_line_segments_reveals_invisible_and_bidi_control_chars() {
        let line = "Safe\u{200B}text\u{202E}hidden\u{FEFF}tag\u{E0001}\u{E0041}vs\u{FE0F}\u{E0100}fmt\u{061C}\u{06DD}\u{2062}\u{FFF9}\u{3164}\u{2028}\u{0007}中文";
        assert_eq!(
            source_line_segments(line),
            vec![
                SourceSegment::Text("Safe".into()),
                SourceSegment::Marker("[U+200B]".into()),
                SourceSegment::Text("text".into()),
                SourceSegment::Marker("[U+202E]".into()),
                SourceSegment::Text("hidden".into()),
                SourceSegment::Marker("[U+FEFF]".into()),
                SourceSegment::Text("tag".into()),
                SourceSegment::Marker("[U+E0001]".into()),
                SourceSegment::Marker("[U+E0041]".into()),
                SourceSegment::Text("vs".into()),
                SourceSegment::Marker("[U+FE0F]".into()),
                SourceSegment::Marker("[U+E0100]".into()),
                SourceSegment::Text("fmt".into()),
                SourceSegment::Marker("[U+061C]".into()),
                SourceSegment::Marker("[U+06DD]".into()),
                SourceSegment::Marker("[U+2062]".into()),
                SourceSegment::Marker("[U+FFF9]".into()),
                SourceSegment::Marker("[U+3164]".into()),
                SourceSegment::Marker("[U+2028]".into()),
                SourceSegment::Marker("[U+0007]".into()),
                SourceSegment::Text("中文".into()),
            ]
        );
    }

    #[test]
    fn bulk_missing_runtimes_groups_two_or_more_uninstalled_slots() {
        let toml = r#"
version = 1
kind = "crew"

[crew]
name = "Trio"

[[roles]]
handle = "coder"
display_name = "Coder"
runtime = "codex"
model = "gpt-5"
effort = "high"

[[roles]]
handle = "reviewer"
display_name = "Reviewer"
runtime = "codex"

[[roles]]
handle = "qa"
display_name = "QA"
runtime = "pi"

[[slots]]
handle = "coder"
role = "coder"

[[slots]]
handle = "reviewer"
role = "reviewer"

[[slots]]
handle = "qa"
role = "qa"
"#;
        let Template::Crew(crew) = parse_template(toml).unwrap() else {
            panic!("expected crew");
        };
        let installed = vec![Runtime::ClaudeCode];
        let mut overrides = BTreeMap::new();
        let bulk = bulk_missing_runtimes(&crew, &overrides, &installed);
        assert_eq!(
            bulk,
            vec![BulkMissingRuntime {
                runtime: Runtime::Codex,
                slot_indices: vec![0, 1],
            }]
        );
        // Switching one slot drops the shared missing count below 2.
        overrides.insert("coder".into(), Runtime::ClaudeCode);
        assert!(bulk_missing_runtimes(&crew, &overrides, &installed).is_empty());

        // Effective summary resets model/effort when switched.
        let summary = effective_slot_summary(
            &crew.slots[0],
            &crew.roles[0],
            Some(Runtime::ClaudeCode),
            &installed,
        );
        assert!(summary.switched);
        assert_eq!(summary.effective_runtime, Runtime::ClaudeCode);
        assert_eq!(summary.model, None);
        assert_eq!(summary.effort, None);
    }

    #[test]
    fn missing_runtime_options_lists_installed_then_divider_and_keep() {
        let opts = missing_runtime_options(&[Runtime::ClaudeCode, Runtime::Pi], Runtime::Codex);
        assert_eq!(opts.len(), 3);
        assert_eq!(opts[0].value, "claude-code");
        assert!(!opts[0].separator_before);
        assert_eq!(opts[1].value, "pi");
        assert_eq!(opts[2].value, "codex");
        assert!(opts[2].separator_before);
        assert!(opts[2].label.contains("Keep Codex"));
    }

    #[test]
    fn extract_error_line_parses_line_number() {
        assert_eq!(extract_error_line("line 12: bad key (line 12)"), Some(12));
        assert_eq!(extract_error_line("no line info"), None);
    }

    #[test]
    fn line_count_copy_uses_singular_for_one_line_and_plural_otherwise() {
        crate::i18n::testing::with_locale(crate::i18n::Locale::English, || {
            assert_eq!(
                crate::i18n::tnf(
                    "template.system_prompt_heading",
                    1,
                    &[("size", "5 B".into())]
                ),
                "SYSTEM PROMPT · 1 LINE · 5 B"
            );
            assert_eq!(
                crate::i18n::tnf(
                    "template.system_prompt_heading",
                    3,
                    &[("size", "24 B".into())]
                ),
                "SYSTEM PROMPT · 3 LINES · 24 B"
            );
            assert_eq!(
                crate::i18n::tnf("template.system_prompt_meta", 1, &[("size", "5 B".into())]),
                "System prompt · 1 line · 5 B"
            );
            assert_eq!(
                crate::i18n::tnf("template.system_prompt_meta", 0, &[("size", "0 B".into())]),
                "System prompt · 0 lines · 0 B"
            );
            assert_eq!(
                crate::i18n::tnf("template.conventions_meta", 1, &[("size", "12 B".into())]),
                "Team conventions · 1 line · 12 B"
            );
            assert_eq!(
                crate::i18n::tnf("template.conventions_meta", 2, &[("size", "24 B".into())]),
                "Team conventions · 2 lines · 24 B"
            );
        });
        crate::i18n::testing::with_locale(crate::i18n::Locale::SimplifiedChinese, || {
            assert_eq!(
                crate::i18n::tnf("template.system_prompt_meta", 1, &[("size", "5 B".into())]),
                "系统提示词 · 1 行 · 5 B"
            );
            assert_eq!(
                crate::i18n::tnf("template.conventions_meta", 2, &[("size", "24 B".into())]),
                "团队约定 · 2 行 · 24 B"
            );
        });
    }
}
