use std::collections::{HashMap, HashSet};
use std::io::Read;

use toml_edit::{Document, Item, TableLike};

use crate::protocol::CodexSpeed;
use crate::slot_handle::validate_slot_handle;
use crate::{Runtime, MAX_SYSTEM_PROMPT_BYTES};

use super::{
    is_agent_runtime, supports_codex_speed, CrewSection, CrewTemplate, RoleTemplate, SlotTemplate,
    StandaloneRoleTemplate, Template, TemplateError, TemplateKind, TemplateMeta,
    MAX_TEMPLATE_BYTES, TEMPLATE_VERSION,
};

const ROLE_KEYS: &[&str] = &[
    "handle",
    "display_name",
    "runtime",
    "model",
    "effort",
    "codex_speed",
    "system_prompt",
];

const SLOT_KEYS: &[&str] = &[
    "handle",
    "role",
    "runtime",
    "model",
    "effort",
    "codex_speed",
];

const CREW_KEYS: &[&str] = &["name", "conventions"];

const TOP_LEVEL_CREW_KEYS: &[&str] = &["version", "kind", "meta", "crew", "slots", "roles"];
const TOP_LEVEL_ROLE_KEYS: &[&str] = &["version", "kind", "meta", "role"];

/// Reads at most `MAX_TEMPLATE_BYTES + 1` bytes from `reader`, rejecting
/// oversized templates and invalid UTF-8 before allocating unbounded memory or
/// crossing IPC.
pub fn read_template_bounded(reader: impl Read) -> Result<String, TemplateError> {
    let mut buf = Vec::new();
    let mut limited = reader.take((MAX_TEMPLATE_BYTES + 1) as u64);
    limited
        .read_to_end(&mut buf)
        .map_err(|err| TemplateError::new(format!("Failed to read template: {err}"), None))?;
    if buf.len() > MAX_TEMPLATE_BYTES {
        return Err(TemplateError::new(
            format!(
                "Template file exceeds the {} KiB size limit.",
                MAX_TEMPLATE_BYTES / 1024
            ),
            None,
        ));
    }
    String::from_utf8(buf)
        .map_err(|_| TemplateError::new("Template file must be valid UTF-8.", None))
}

pub fn parse_template(input: &str) -> Result<Template, TemplateError> {
    if input.len() > MAX_TEMPLATE_BYTES {
        return Err(TemplateError::new(
            format!(
                "Template file is {} bytes; maximum allowed size is {} KiB.",
                input.len(),
                MAX_TEMPLATE_BYTES / 1024
            ),
            None,
        ));
    }

    let doc = input.parse::<Document<String>>().map_err(|err| {
        let line = err.span().map(|span| line_at(input, span.start));
        TemplateError::new(format!("Invalid TOML: {}", err.message()), line)
    })?;

    let root = doc.as_table();

    let version_item = root.get("version").ok_or_else(|| {
        TemplateError::new(
            "Missing required field `version` (expected version = 1).",
            None,
        )
    })?;
    let version_line = item_line(input, version_item);
    let version = version_item.as_integer().ok_or_else(|| {
        TemplateError::new(
            "Field `version` must be an integer (expected version = 1).",
            version_line,
        )
    })?;
    if version != i64::from(TEMPLATE_VERSION) {
        return Err(TemplateError::new(
            format!("Unsupported template version {version}; update Runner to import this file."),
            version_line,
        ));
    }

    let kind_item = root.get("kind").ok_or_else(|| {
        TemplateError::new(
            "Missing required field `kind` (expected \"crew\" or \"role\").",
            None,
        )
    })?;
    let kind_line = item_line(input, kind_item);
    let kind_str = kind_item.as_str().ok_or_else(|| {
        TemplateError::new(
            "Field `kind` must be a string (\"crew\" or \"role\").",
            kind_line,
        )
    })?;
    let kind = match kind_str {
        "crew" => TemplateKind::Crew,
        "role" => TemplateKind::Role,
        other => {
            return Err(TemplateError::new(
                format!("Unsupported template kind {other:?}; expected \"crew\" or \"role\"."),
                kind_line,
            ));
        }
    };

    let allowed_top = match kind {
        TemplateKind::Crew => TOP_LEVEL_CREW_KEYS,
        TemplateKind::Role => TOP_LEVEL_ROLE_KEYS,
    };
    for (key, item) in root.iter() {
        if !allowed_top.contains(&key) {
            let line = table_key_line(input, root, key).or_else(|| item_line(input, item));
            let msg = match (kind, key) {
                (TemplateKind::Crew, "role") => {
                    "Section `[role]` is not allowed in a crew template (`kind = \"crew\"`)."
                        .to_owned()
                }
                (TemplateKind::Role, "crew") => {
                    "Section `[crew]` is not allowed in a role template (`kind = \"role\"`)."
                        .to_owned()
                }
                (TemplateKind::Role, "slots") => {
                    "Section `[[slots]]` is not allowed in a role template (`kind = \"role\"`)."
                        .to_owned()
                }
                (TemplateKind::Role, "roles") => {
                    "Section `[[roles]]` is not allowed in a role template (`kind = \"role\"`)."
                        .to_owned()
                }
                _ => format!("Unknown key `{key}` at template root."),
            };
            return Err(TemplateError::new(msg, line));
        }
    }

    let meta = match root.get("meta") {
        Some(item) => parse_meta(input, item)?,
        None => TemplateMeta::default(),
    };

    match kind {
        TemplateKind::Crew => parse_crew_template(input, root, meta).map(Template::Crew),
        TemplateKind::Role => parse_role_template_doc(input, root, meta).map(Template::Role),
    }
}

fn parse_meta(input: &str, item: &Item) -> Result<TemplateMeta, TemplateError> {
    let table = item.as_table_like().ok_or_else(|| {
        TemplateError::new(
            "Section `[meta]` must be a TOML table.",
            item_line(input, item),
        )
    })?;

    let description = parse_optional_trimmed_string(input, table, "description", "[meta]")?;
    let author = parse_optional_trimmed_string(input, table, "author", "[meta]")?;
    let homepage = parse_optional_trimmed_string(input, table, "homepage", "[meta]")?;
    let tags = match table.get("tags") {
        None => Vec::new(),
        Some(tags_item) => {
            let line = item_line(input, tags_item);
            let array = tags_item.as_array().ok_or_else(|| {
                TemplateError::new("Field `tags` in [meta] must be an array of strings.", line)
            })?;
            let mut out = Vec::with_capacity(array.len());
            for elem in array.iter() {
                let s = elem.as_str().ok_or_else(|| {
                    TemplateError::new(
                        "Every item in `[meta].tags` must be a string.",
                        elem.span().map(|s| line_at(input, s.start)).or(line),
                    )
                })?;
                let trimmed = s.trim();
                if !trimmed.is_empty() {
                    out.push(trimmed.to_owned());
                }
            }
            out
        }
    };

    Ok(TemplateMeta {
        description,
        author,
        tags,
        homepage,
    })
}

fn parse_crew_template(
    input: &str,
    root: &dyn TableLike,
    meta: TemplateMeta,
) -> Result<CrewTemplate, TemplateError> {
    let crew_item = root.get("crew").ok_or_else(|| {
        TemplateError::new(
            "Crew template requires a `[crew]` section with `name`.",
            None,
        )
    })?;
    let crew_table = crew_item.as_table_like().ok_or_else(|| {
        TemplateError::new(
            "Section `[crew]` must be a TOML table.",
            item_line(input, crew_item),
        )
    })?;
    reject_unknown_keys(input, crew_table, CREW_KEYS, "[crew]")?;

    let name = parse_required_trimmed_string(input, crew_table, "name", "[crew]")?;
    let conventions = parse_optional_preserved_string(input, crew_table, "conventions", "[crew]")?;
    let crew = CrewSection { name, conventions };

    let role_tables = extract_table_array(input, root, "roles", "[[roles]]")?;
    let mut roles = Vec::with_capacity(role_tables.len());
    let mut role_by_handle: HashMap<String, (Runtime, Option<usize>)> = HashMap::new();

    for (index, (table, table_line)) in role_tables.into_iter().enumerate() {
        let fallback_context = format!("roles[{index}]");
        let role = parse_role_table(input, table, &fallback_context, table_line)?;
        if role_by_handle.contains_key(&role.handle) {
            let line = table_key_line(input, table, "handle").or(table_line);
            return Err(TemplateError::new(
                format!("Duplicate role handle `@{}` in [[roles]].", role.handle),
                line,
            ));
        }
        let line = table_key_line(input, table, "handle").or(table_line);
        role_by_handle.insert(role.handle.clone(), (role.runtime, line));
        roles.push(role);
    }

    let slot_tables = extract_table_array(input, root, "slots", "[[slots]]")?;
    let mut slots = Vec::with_capacity(slot_tables.len());
    let mut seen_slot_handles = HashSet::new();
    let mut used_roles = HashSet::new();

    for (index, (table, table_line)) in slot_tables.into_iter().enumerate() {
        let fallback_context = format!("slots[{index}]");
        let slot = parse_slot_table(input, table, &fallback_context, table_line, &role_by_handle)?;
        if !seen_slot_handles.insert(slot.handle.clone()) {
            let line = table_key_line(input, table, "handle").or(table_line);
            return Err(TemplateError::new(
                format!("Duplicate slot handle `@{}` in [[slots]].", slot.handle),
                line,
            ));
        }
        used_roles.insert(slot.role.clone());
        slots.push(slot);
    }

    for role in &roles {
        if !used_roles.contains(&role.handle) {
            let line = role_by_handle.get(&role.handle).and_then(|(_, line)| *line);
            return Err(TemplateError::new(
                format!(
                    "Unused role `@{}` in [[roles]]; every role in a crew template must be referenced by a slot.",
                    role.handle
                ),
                line,
            ));
        }
    }

    Ok(CrewTemplate {
        meta,
        crew,
        slots,
        roles,
    })
}

fn parse_role_template_doc(
    input: &str,
    root: &dyn TableLike,
    meta: TemplateMeta,
) -> Result<StandaloneRoleTemplate, TemplateError> {
    let role_item = root
        .get("role")
        .ok_or_else(|| TemplateError::new("Role template requires a `[role]` section.", None))?;
    let role_line = item_line(input, role_item);
    let role_table = role_item
        .as_table_like()
        .ok_or_else(|| TemplateError::new("Section `[role]` must be a TOML table.", role_line))?;
    let role = parse_role_table(input, role_table, "[role]", role_line)?;
    Ok(StandaloneRoleTemplate { meta, role })
}

type TableEntry<'a> = (&'a dyn TableLike, Option<usize>);

fn extract_table_array<'a>(
    input: &str,
    root: &'a dyn TableLike,
    key: &str,
    section_label: &str,
) -> Result<Vec<TableEntry<'a>>, TemplateError> {
    let Some(item) = root.get(key) else {
        return Ok(Vec::new());
    };
    let line = item_line(input, item);
    if let Some(aot) = item.as_array_of_tables() {
        return Ok(aot
            .iter()
            .map(|table| {
                let tline = table.span().map(|s| line_at(input, s.start)).or(line);
                (table as &dyn TableLike, tline)
            })
            .collect());
    }
    if let Some(array) = item.as_array() {
        let mut out = Vec::with_capacity(array.len());
        for value in array.iter() {
            let vline = value.span().map(|s| line_at(input, s.start)).or(line);
            let inline = value.as_inline_table().ok_or_else(|| {
                TemplateError::new(
                    format!("Section `{section_label}` entries must be tables."),
                    vline,
                )
            })?;
            out.push((inline as &dyn TableLike, vline));
        }
        return Ok(out);
    }
    Err(TemplateError::new(
        format!("Section `{section_label}` must be an array of tables."),
        line,
    ))
}

fn parse_role_table(
    input: &str,
    table: &dyn TableLike,
    fallback_context: &str,
    table_line: Option<usize>,
) -> Result<RoleTemplate, TemplateError> {
    let context = table
        .get("handle")
        .and_then(Item::as_str)
        .filter(|h| !h.is_empty())
        .map(|h| format!("role @{h}"))
        .unwrap_or_else(|| fallback_context.to_owned());

    reject_unknown_keys(input, table, ROLE_KEYS, &context)?;

    let handle = parse_handle(input, table, "handle", &context, table_line)?;
    let context = format!("role @{handle}");

    let display_name = parse_required_trimmed_string(input, table, "display_name", &context)?;
    let runtime = parse_required_runtime(input, table, &context, table_line)?;
    let model = parse_optional_trimmed_string(input, table, "model", &context)?;
    let effort = parse_optional_trimmed_string(input, table, "effort", &context)?;
    let codex_speed = parse_optional_codex_speed(input, table, runtime, &context)?;
    let system_prompt = parse_optional_system_prompt(input, table, &context)?;

    Ok(RoleTemplate {
        handle,
        display_name,
        runtime,
        model,
        effort,
        codex_speed,
        system_prompt,
    })
}

fn parse_slot_table(
    input: &str,
    table: &dyn TableLike,
    fallback_context: &str,
    table_line: Option<usize>,
    role_by_handle: &HashMap<String, (Runtime, Option<usize>)>,
) -> Result<SlotTemplate, TemplateError> {
    let context = table
        .get("handle")
        .and_then(Item::as_str)
        .filter(|h| !h.is_empty())
        .map(|h| format!("slot @{h}"))
        .unwrap_or_else(|| fallback_context.to_owned());

    reject_unknown_keys(input, table, SLOT_KEYS, &context)?;

    let handle = parse_handle(input, table, "handle", &context, table_line)?;
    let context = format!("slot @{handle}");

    let role_item = table.get("role").ok_or_else(|| {
        TemplateError::new(
            format!("Missing required field `role` on {context}."),
            table_line,
        )
    })?;
    let role_line = item_line(input, role_item).or(table_line);
    let role = role_item
        .as_str()
        .ok_or_else(|| {
            TemplateError::new(
                format!("Field `role` on {context} must be a string."),
                role_line,
            )
        })?
        .trim()
        .to_owned();
    if role.is_empty() {
        return Err(TemplateError::new(
            format!("Field `role` on {context} must not be empty."),
            role_line,
        ));
    }
    let &(role_runtime, _) = role_by_handle.get(&role).ok_or_else(|| {
        TemplateError::new(
            format!(
                "Slot `@{handle}` references unknown role `@{role}` (not defined in [[roles]])."
            ),
            role_line,
        )
    })?;

    let runtime = parse_optional_runtime(input, table, &context)?;
    let effective_runtime = runtime.unwrap_or(role_runtime);
    let model = parse_optional_trimmed_string(input, table, "model", &context)?;
    let effort = parse_optional_trimmed_string(input, table, "effort", &context)?;
    let codex_speed = parse_optional_codex_speed(input, table, effective_runtime, &context)?;

    Ok(SlotTemplate {
        handle,
        role,
        runtime,
        model,
        effort,
        codex_speed,
    })
}

fn reject_unknown_keys(
    input: &str,
    table: &dyn TableLike,
    allowed: &[&str],
    context: &str,
) -> Result<(), TemplateError> {
    for (key, item) in table.iter() {
        if !allowed.contains(&key) {
            let line = table_key_line(input, table, key).or_else(|| item_line(input, item));
            return Err(TemplateError::new(
                format!("Unknown or excluded key `{key}` on {context}."),
                line,
            ));
        }
    }
    Ok(())
}

fn parse_handle(
    input: &str,
    table: &dyn TableLike,
    key: &str,
    context: &str,
    table_line: Option<usize>,
) -> Result<String, TemplateError> {
    let item = table.get(key).ok_or_else(|| {
        TemplateError::new(
            format!("Missing required field `{key}` on {context}."),
            table_line,
        )
    })?;
    let line = item_line(input, item).or(table_line);
    let raw = item.as_str().ok_or_else(|| {
        TemplateError::new(
            format!("Field `{key}` on {context} must be a string."),
            line,
        )
    })?;
    if raw.is_empty() {
        return Err(TemplateError::new(
            format!("Field `{key}` on {context} must not be empty."),
            line,
        ));
    }
    if let Some(reason) = validate_slot_handle(raw) {
        return Err(TemplateError::new(
            format!("Invalid handle {raw:?} on {context}: {reason}"),
            line,
        ));
    }
    Ok(raw.to_owned())
}

fn parse_required_trimmed_string(
    input: &str,
    table: &dyn TableLike,
    key: &str,
    context: &str,
) -> Result<String, TemplateError> {
    let item = table.get(key).ok_or_else(|| {
        TemplateError::new(
            format!("Missing required field `{key}` on {context}."),
            None,
        )
    })?;
    let line = item_line(input, item);
    let raw = item.as_str().ok_or_else(|| {
        TemplateError::new(
            format!("Field `{key}` on {context} must be a string."),
            line,
        )
    })?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(TemplateError::new(
            format!("Field `{key}` on {context} must not be empty."),
            line,
        ));
    }
    Ok(trimmed.to_owned())
}

fn parse_optional_trimmed_string(
    input: &str,
    table: &dyn TableLike,
    key: &str,
    context: &str,
) -> Result<Option<String>, TemplateError> {
    let Some(item) = table.get(key) else {
        return Ok(None);
    };
    let line = item_line(input, item);
    let raw = item.as_str().ok_or_else(|| {
        TemplateError::new(
            format!("Field `{key}` on {context} must be a string."),
            line,
        )
    })?;
    let trimmed = raw.trim();
    Ok((!trimmed.is_empty()).then(|| trimmed.to_owned()))
}

fn parse_optional_preserved_string(
    input: &str,
    table: &dyn TableLike,
    key: &str,
    context: &str,
) -> Result<Option<String>, TemplateError> {
    let Some(item) = table.get(key) else {
        return Ok(None);
    };
    let line = item_line(input, item);
    let raw = item.as_str().ok_or_else(|| {
        TemplateError::new(
            format!("Field `{key}` on {context} must be a string."),
            line,
        )
    })?;
    Ok((!raw.trim().is_empty()).then(|| raw.to_owned()))
}

fn parse_optional_system_prompt(
    input: &str,
    table: &dyn TableLike,
    context: &str,
) -> Result<Option<String>, TemplateError> {
    let Some(item) = table.get("system_prompt") else {
        return Ok(None);
    };
    let line = item_line(input, item);
    let raw = item.as_str().ok_or_else(|| {
        TemplateError::new(
            format!("Field `system_prompt` on {context} must be a string."),
            line,
        )
    })?;
    if raw.len() > MAX_SYSTEM_PROMPT_BYTES {
        return Err(TemplateError::new(
            format!(
                "Field `system_prompt` on {context} is {} bytes; maximum allowed is {} KiB.",
                raw.len(),
                MAX_SYSTEM_PROMPT_BYTES / 1024
            ),
            line,
        ));
    }
    Ok((!raw.is_empty()).then(|| raw.to_owned()))
}

fn parse_required_runtime(
    input: &str,
    table: &dyn TableLike,
    context: &str,
    table_line: Option<usize>,
) -> Result<Runtime, TemplateError> {
    let item = table.get("runtime").ok_or_else(|| {
        TemplateError::new(
            format!("Missing required field `runtime` on {context}."),
            table_line,
        )
    })?;
    let line = item_line(input, item).or(table_line);
    let raw = item.as_str().ok_or_else(|| {
        TemplateError::new(
            format!("Field `runtime` on {context} must be a string."),
            line,
        )
    })?;
    parse_agent_runtime_str(raw, context, line)
}

fn parse_optional_runtime(
    input: &str,
    table: &dyn TableLike,
    context: &str,
) -> Result<Option<Runtime>, TemplateError> {
    let Some(item) = table.get("runtime") else {
        return Ok(None);
    };
    let line = item_line(input, item);
    let raw = item.as_str().ok_or_else(|| {
        TemplateError::new(
            format!("Field `runtime` on {context} must be a string."),
            line,
        )
    })?;
    parse_agent_runtime_str(raw, context, line).map(Some)
}

fn parse_agent_runtime_str(
    raw: &str,
    context: &str,
    line: Option<usize>,
) -> Result<Runtime, TemplateError> {
    Runtime::parse(raw)
        .filter(|rt| is_agent_runtime(*rt))
        .ok_or_else(|| TemplateError::new(format!("Unknown runtime \"{raw}\" on {context}."), line))
}

fn parse_optional_codex_speed(
    input: &str,
    table: &dyn TableLike,
    effective_runtime: Runtime,
    context: &str,
) -> Result<Option<CodexSpeed>, TemplateError> {
    let Some(item) = table.get("codex_speed") else {
        return Ok(None);
    };
    let line = item_line(input, item);
    let raw = item.as_str().ok_or_else(|| {
        TemplateError::new(
            format!("Field `codex_speed` on {context} must be \"standard\" or \"fast\"."),
            line,
        )
    })?;
    let speed = match raw {
        "standard" => CodexSpeed::Standard,
        "fast" => CodexSpeed::Fast,
        other => {
            return Err(TemplateError::new(
                format!(
                    "Invalid `codex_speed` {other:?} on {context}; expected \"standard\" or \"fast\"."
                ),
                line,
            ));
        }
    };
    if !supports_codex_speed(effective_runtime) {
        return Err(TemplateError::new(
            format!(
                "Field `codex_speed` is not supported on {context} (runtime \"{}\").",
                effective_runtime.key()
            ),
            line,
        ));
    }
    Ok(Some(speed))
}

fn line_at(input: &str, byte_offset: usize) -> usize {
    let clamped = byte_offset.min(input.len());
    input[..clamped].bytes().filter(|&b| b == b'\n').count() + 1
}

fn item_line(input: &str, item: &Item) -> Option<usize> {
    item.span().map(|span| line_at(input, span.start))
}

fn table_key_line(input: &str, table: &dyn TableLike, key: &str) -> Option<usize> {
    table
        .key(key)
        .and_then(|k| k.span())
        .map(|span| line_at(input, span.start))
}
