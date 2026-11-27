use std::collections::HashSet;

use crate::protocol::{CodexSpeed, Crew, Role, SlotWithRole};
use crate::Runtime;

use super::{
    is_agent_runtime, parse_template, CrewSection, CrewTemplate, ExportedTemplate, RoleTemplate,
    SlotTemplate, StandaloneRoleTemplate, Template, TemplateError, TemplateMeta, TEMPLATE_VERSION,
};

pub fn slugify(name: &str) -> String {
    let mut slug = String::new();
    let mut prev_dash = false;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !slug.is_empty() && !prev_dash {
            slug.push('-');
            prev_dash = true;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        "crew".to_owned()
    } else {
        slug
    }
}

pub fn crew_filename(name: &str) -> String {
    format!("{}.crew.toml", slugify(name))
}

pub fn role_filename(handle: &str) -> String {
    format!("{}.role.toml", slugify(handle))
}

pub fn export_crew(crew: &Crew, slots: &[SlotWithRole]) -> Result<ExportedTemplate, TemplateError> {
    let mut ordered_slots: Vec<&SlotWithRole> = slots.iter().collect();
    ordered_slots.sort_by_key(|entry| entry.slot.position);

    let mut slot_templates = Vec::with_capacity(ordered_slots.len());
    let mut role_templates = Vec::new();
    let mut seen_role_ids = HashSet::new();

    for entry in ordered_slots {
        let slot = &entry.slot;
        let role = &entry.role;

        if seen_role_ids.insert(role.id.clone()) {
            role_templates.push(role_to_template(role)?);
        }

        let runtime = match slot
            .runtime_override
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(raw) => Some(parse_export_runtime(
                raw,
                &format!("slot @{}", slot.slot_handle),
            )?),
            None => None,
        };

        slot_templates.push(SlotTemplate {
            handle: slot.slot_handle.clone(),
            role: role.handle.clone(),
            runtime,
            model: non_empty_trimmed(slot.model_override.as_deref()),
            effort: non_empty_trimmed(slot.effort_override.as_deref()),
            codex_speed: slot.codex_speed_override,
        });
    }

    let template = Template::Crew(CrewTemplate {
        meta: TemplateMeta::default(),
        crew: CrewSection {
            name: crew.name.trim().to_owned(),
            conventions: crew
                .system_prompt_addendum
                .as_ref()
                .filter(|s| !s.trim().is_empty())
                .cloned(),
        },
        slots: slot_templates,
        roles: role_templates,
    });

    let toml = format_template(&template)?;
    Ok(ExportedTemplate {
        filename: crew_filename(&crew.name),
        toml,
    })
}

pub fn export_role(role: &Role) -> Result<ExportedTemplate, TemplateError> {
    let template = Template::Role(StandaloneRoleTemplate {
        meta: TemplateMeta::default(),
        role: role_to_template(role)?,
    });
    let toml = format_template(&template)?;
    Ok(ExportedTemplate {
        filename: role_filename(&role.handle),
        toml,
    })
}

fn role_to_template(role: &Role) -> Result<RoleTemplate, TemplateError> {
    let runtime = parse_export_runtime(&role.runtime, &format!("role @{}", role.handle))?;
    Ok(RoleTemplate {
        handle: role.handle.clone(),
        display_name: role.display_name.trim().to_owned(),
        runtime,
        model: non_empty_trimmed(role.model.as_deref()),
        effort: non_empty_trimmed(role.effort.as_deref()),
        codex_speed: role.codex_speed,
        system_prompt: role
            .system_prompt
            .as_ref()
            .filter(|s| !s.is_empty())
            .cloned(),
    })
}

fn parse_export_runtime(raw: &str, context: &str) -> Result<Runtime, TemplateError> {
    Runtime::parse(raw)
        .filter(|rt| is_agent_runtime(*rt))
        .ok_or_else(|| TemplateError::new(format!("Unknown runtime \"{raw}\" on {context}."), None))
}

fn non_empty_trimmed(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// Formats a template into readable TOML v1 and self-validates the emitted
/// output with the import validator so every exported document is guaranteed
/// to import cleanly.
pub fn format_template(template: &Template) -> Result<String, TemplateError> {
    let mut out = String::new();
    out.push_str(&format!("version = {TEMPLATE_VERSION}\n"));
    out.push_str(&format!(
        "kind = {}\n",
        format_basic_string(template.kind().as_str())
    ));

    let meta = template.meta();
    if !meta.is_empty() {
        out.push_str("\n[meta]\n");
        if let Some(description) = meta.description.as_deref().filter(|s| !s.is_empty()) {
            out.push_str(&format!(
                "description = {}\n",
                format_toml_string(description)
            ));
        }
        if let Some(author) = meta.author.as_deref().filter(|s| !s.is_empty()) {
            out.push_str(&format!("author = {}\n", format_basic_string(author)));
        }
        if !meta.tags.is_empty() {
            let formatted = meta
                .tags
                .iter()
                .map(|tag| format_basic_string(tag))
                .collect::<Vec<_>>()
                .join(", ");
            out.push_str(&format!("tags = [{formatted}]\n"));
        }
        if let Some(homepage) = meta.homepage.as_deref().filter(|s| !s.is_empty()) {
            out.push_str(&format!("homepage = {}\n", format_basic_string(homepage)));
        }
    }

    match template {
        Template::Crew(crew) => {
            out.push_str("\n[crew]\n");
            out.push_str(&format!(
                "name = {}\n",
                format_basic_string(&crew.crew.name)
            ));
            if let Some(conventions) = crew.crew.conventions.as_deref().filter(|s| !s.is_empty()) {
                out.push_str(&format!(
                    "conventions = {}\n",
                    format_toml_string(conventions)
                ));
            }

            for slot in &crew.slots {
                out.push_str("\n[[slots]]\n");
                out.push_str(&format!("handle = {}\n", format_basic_string(&slot.handle)));
                out.push_str(&format!("role = {}\n", format_basic_string(&slot.role)));
                if let Some(runtime) = slot.runtime {
                    out.push_str(&format!(
                        "runtime = {}\n",
                        format_basic_string(runtime.key())
                    ));
                }
                if let Some(model) = slot.model.as_deref().filter(|s| !s.is_empty()) {
                    out.push_str(&format!("model = {}\n", format_basic_string(model)));
                }
                if let Some(effort) = slot.effort.as_deref().filter(|s| !s.is_empty()) {
                    out.push_str(&format!("effort = {}\n", format_basic_string(effort)));
                }
                if let Some(speed) = slot.codex_speed {
                    out.push_str(&format!(
                        "codex_speed = {}\n",
                        format_basic_string(codex_speed_str(speed))
                    ));
                }
            }

            for role in &crew.roles {
                out.push_str("\n[[roles]]\n");
                write_role_fields(&mut out, role);
            }
        }
        Template::Role(standalone) => {
            out.push_str("\n[role]\n");
            write_role_fields(&mut out, &standalone.role);
        }
    }

    // Self-validate through the import parser so export never produces a file
    // that fails validation on import.
    parse_template(&out)?;
    Ok(out)
}

fn write_role_fields(out: &mut String, role: &RoleTemplate) {
    out.push_str(&format!("handle = {}\n", format_basic_string(&role.handle)));
    out.push_str(&format!(
        "display_name = {}\n",
        format_basic_string(&role.display_name)
    ));
    out.push_str(&format!(
        "runtime = {}\n",
        format_basic_string(role.runtime.key())
    ));
    if let Some(model) = role.model.as_deref().filter(|s| !s.is_empty()) {
        out.push_str(&format!("model = {}\n", format_basic_string(model)));
    }
    if let Some(effort) = role.effort.as_deref().filter(|s| !s.is_empty()) {
        out.push_str(&format!("effort = {}\n", format_basic_string(effort)));
    }
    if let Some(speed) = role.codex_speed {
        out.push_str(&format!(
            "codex_speed = {}\n",
            format_basic_string(codex_speed_str(speed))
        ));
    }
    if let Some(prompt) = role.system_prompt.as_deref().filter(|s| !s.is_empty()) {
        out.push_str(&format!("system_prompt = {}\n", format_toml_string(prompt)));
    }
}

fn codex_speed_str(speed: CodexSpeed) -> &'static str {
    match speed {
        CodexSpeed::Standard => "standard",
        CodexSpeed::Fast => "fast",
    }
}

fn format_basic_string(s: &str) -> String {
    toml_edit::Value::from(s).to_string()
}

fn format_toml_string(s: &str) -> String {
    if !s.contains('\n') {
        return format_basic_string(s);
    }
    // Multiline basic string: TOML trims the single newline immediately after
    // the opening `"""`, so emitting `"""\n<escaped>"""` round-trips `s`
    // byte-for-byte while keeping multiline Markdown readable and diffable.
    let mut out = String::with_capacity(s.len() + 16);
    out.push_str("\"\"\"\n");
    let chars: Vec<char> = s.chars().collect();
    let mut consecutive_quotes = 0usize;
    for (idx, &ch) in chars.iter().enumerate() {
        match ch {
            '\\' => {
                consecutive_quotes = 0;
                out.push_str("\\\\");
            }
            '"' => {
                let remaining_all_quotes = chars[idx..].iter().all(|&c| c == '"');
                if consecutive_quotes >= 2 || remaining_all_quotes {
                    out.push_str("\\\"");
                    consecutive_quotes = 0;
                } else {
                    out.push('"');
                    consecutive_quotes += 1;
                }
            }
            '\n' => {
                consecutive_quotes = 0;
                out.push('\n');
            }
            '\t' => {
                consecutive_quotes = 0;
                out.push('\t');
            }
            '\r' => {
                consecutive_quotes = 0;
                out.push_str("\\r");
            }
            '\u{0008}' => {
                consecutive_quotes = 0;
                out.push_str("\\b");
            }
            '\u{000C}' => {
                consecutive_quotes = 0;
                out.push_str("\\f");
            }
            c if (c as u32) < 0x20 || c == '\u{007F}' => {
                consecutive_quotes = 0;
                out.push_str(&format!("\\u{:04X}", c as u32));
            }
            c => {
                consecutive_quotes = 0;
                out.push(c);
            }
        }
    }
    out.push_str("\"\"\"");
    out
}
