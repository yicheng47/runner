use std::collections::{BTreeMap, HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::protocol::CodexSpeed;
use crate::Runtime;

use super::{
    is_agent_runtime, supports_codex_speed, CrewTemplate, RoleTemplate, StandaloneRoleTemplate,
    Template, TemplateError, TEMPLATE_VERSION,
};

const MAX_HANDLE_LEN: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum TemplatePreview {
    Valid(Box<TemplatePreviewPlan>),
    Invalid { error: TemplateError },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplatePreviewPlan {
    pub version: u32,
    pub template: Template,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggested_crew_name: Option<String>,
    pub planned_roles: Vec<PlannedRole>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub planned_slots: Vec<PlannedSlot>,
    pub installed_runtimes: Vec<Runtime>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedRole {
    pub template_handle: String,
    pub planned_handle: String,
    pub auto_suffixed: bool,
    pub display_name: String,
    pub runtime: Runtime,
    pub runtime_installed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex_speed: Option<CodexSpeed>,
    pub prompt_bytes: usize,
    pub prompt_lines: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedSlot {
    pub slot_handle: String,
    pub template_role_handle: String,
    pub planned_role_handle: String,
    pub effective_runtime: Runtime,
    pub runtime_installed: bool,
    pub has_slot_override: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedSlot {
    pub slot_handle: String,
    pub template_role_handle: String,
    pub runtime_override: Option<Runtime>,
    pub model_override: Option<String>,
    pub effort_override: Option<String>,
    pub codex_speed_override: Option<CodexSpeed>,
    pub effective_runtime: Runtime,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedCrewImport {
    pub roles: Vec<RoleTemplate>,
    pub slots: Vec<ResolvedSlot>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRoleImport {
    pub role: RoleTemplate,
}

/// Allocates the first free role handle within 32 characters (`<base>`,
/// `<base>-2`, `<base>-3`, …), inserting the chosen handle into `used`.
pub fn allocate_role_handle(base: &str, used: &mut HashSet<String>) -> String {
    if used.insert(base.to_owned()) {
        return base.to_owned();
    }
    for n in 2u64.. {
        let suffix = format!("-{n}");
        let max_base = MAX_HANDLE_LEN.saturating_sub(suffix.len());
        let prefix = &base[..base.len().min(max_base)];
        let candidate = format!("{prefix}{suffix}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
    }
    unreachable!()
}

/// Suggests `<name>` if unused (case-insensitive), otherwise `<name> (2)`,
/// `<name> (3)`, …
pub fn suggest_crew_name(base: &str, existing_names: &[String]) -> String {
    let base = base.trim();
    let taken = |candidate: &str| {
        existing_names
            .iter()
            .any(|name| name.trim().eq_ignore_ascii_case(candidate))
    };
    if !taken(base) {
        return base.to_owned();
    }
    for n in 2u64.. {
        let candidate = format!("{base} ({n})");
        if !taken(&candidate) {
            return candidate;
        }
    }
    unreachable!()
}

pub fn prompt_stats(prompt: Option<&str>) -> (usize, usize) {
    match prompt {
        Some(text) if !text.is_empty() => (text.len(), text.lines().count().max(1)),
        _ => (0, 0),
    }
}

pub fn build_preview(
    template: Template,
    existing_crew_names: &[String],
    existing_role_handles: &HashSet<String>,
    installed_runtimes: &[Runtime],
) -> TemplatePreviewPlan {
    let installed_set: HashSet<Runtime> = installed_runtimes.iter().copied().collect();
    let mut used_handles = existing_role_handles.clone();

    match &template {
        Template::Crew(crew) => {
            let suggested_crew_name = Some(suggest_crew_name(&crew.crew.name, existing_crew_names));
            let mut planned_roles = Vec::with_capacity(crew.roles.len());
            let mut role_map: HashMap<&str, (&RoleTemplate, String)> = HashMap::new();

            for role in &crew.roles {
                let planned_handle = allocate_role_handle(&role.handle, &mut used_handles);
                let auto_suffixed = planned_handle != role.handle;
                let (prompt_bytes, prompt_lines) = prompt_stats(role.system_prompt.as_deref());
                role_map.insert(role.handle.as_str(), (role, planned_handle.clone()));
                planned_roles.push(PlannedRole {
                    template_handle: role.handle.clone(),
                    planned_handle,
                    auto_suffixed,
                    display_name: role.display_name.clone(),
                    runtime: role.runtime,
                    runtime_installed: installed_set.contains(&role.runtime),
                    model: role.model.clone(),
                    effort: role.effort.clone(),
                    codex_speed: role.codex_speed,
                    prompt_bytes,
                    prompt_lines,
                });
            }

            let mut planned_slots = Vec::with_capacity(crew.slots.len());
            for slot in &crew.slots {
                let (role, planned_role_handle) = &role_map[slot.role.as_str()];
                let effective_runtime = slot.runtime.unwrap_or(role.runtime);
                let has_slot_override = slot.runtime.is_some()
                    || slot.model.is_some()
                    || slot.effort.is_some()
                    || slot.codex_speed.is_some();
                planned_slots.push(PlannedSlot {
                    slot_handle: slot.handle.clone(),
                    template_role_handle: slot.role.clone(),
                    planned_role_handle: planned_role_handle.clone(),
                    effective_runtime,
                    runtime_installed: installed_set.contains(&effective_runtime),
                    has_slot_override,
                });
            }

            TemplatePreviewPlan {
                version: TEMPLATE_VERSION,
                template,
                suggested_crew_name,
                planned_roles,
                planned_slots,
                installed_runtimes: installed_runtimes.to_vec(),
            }
        }
        Template::Role(standalone) => {
            let role = &standalone.role;
            let planned_handle = allocate_role_handle(&role.handle, &mut used_handles);
            let auto_suffixed = planned_handle != role.handle;
            let (prompt_bytes, prompt_lines) = prompt_stats(role.system_prompt.as_deref());
            let planned_roles = vec![PlannedRole {
                template_handle: role.handle.clone(),
                planned_handle,
                auto_suffixed,
                display_name: role.display_name.clone(),
                runtime: role.runtime,
                runtime_installed: installed_set.contains(&role.runtime),
                model: role.model.clone(),
                effort: role.effort.clone(),
                codex_speed: role.codex_speed,
                prompt_bytes,
                prompt_lines,
            }];

            TemplatePreviewPlan {
                version: TEMPLATE_VERSION,
                template,
                suggested_crew_name: None,
                planned_roles,
                planned_slots: Vec::new(),
                installed_runtimes: installed_runtimes.to_vec(),
            }
        }
    }
}

/// Applies per-slot runtime overrides (`slot_handle -> target_runtime`) to a
/// crew template, preserving lossless round-trips when untouched and clearing
/// `model`, `effort`, and `codex_speed` on any switched slot/role.
pub fn resolve_crew_import(
    template: &CrewTemplate,
    runtime_overrides: &BTreeMap<String, Runtime>,
) -> Result<ResolvedCrewImport, TemplateError> {
    let valid_slots: HashSet<&str> = template.slots.iter().map(|s| s.handle.as_str()).collect();
    for (slot_handle, &runtime) in runtime_overrides {
        if !valid_slots.contains(slot_handle.as_str()) {
            return Err(TemplateError::new(
                format!("Unknown slot `@{slot_handle}` in runtime_overrides."),
                None,
            ));
        }
        if !is_agent_runtime(runtime) {
            return Err(TemplateError::new(
                format!(
                    "Invalid target runtime \"{}\" for slot @{slot_handle}.",
                    runtime.key()
                ),
                None,
            ));
        }
    }

    let orig_roles: HashMap<&str, &RoleTemplate> = template
        .roles
        .iter()
        .map(|r| (r.handle.as_str(), r))
        .collect();

    // Group slot choices by template role handle in slot order.
    let mut slots_per_role: HashMap<&str, Vec<(usize, Runtime, Runtime, bool)>> = HashMap::new();
    let mut slot_choices: Vec<(Runtime, Runtime, bool)> = Vec::with_capacity(template.slots.len());

    for (idx, slot) in template.slots.iter().enumerate() {
        let orig_role = orig_roles[slot.role.as_str()];
        let orig_effective = slot.runtime.unwrap_or(orig_role.runtime);
        let chosen = runtime_overrides
            .get(&slot.handle)
            .copied()
            .unwrap_or(orig_effective);
        let switched = chosen != orig_effective;
        slot_choices.push((orig_effective, chosen, switched));
        slots_per_role.entry(slot.role.as_str()).or_default().push((
            idx,
            orig_effective,
            chosen,
            switched,
        ));
    }

    let mut resolved_roles = Vec::with_capacity(template.roles.len());
    let mut final_role_runtime: HashMap<&str, Runtime> = HashMap::new();
    let mut role_tuning_cleared: HashMap<&str, bool> = HashMap::new();

    for orig_role in &template.roles {
        let entries = slots_per_role
            .get(orig_role.handle.as_str())
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let any_switched = entries.iter().any(|(_, _, _, switched)| *switched);

        if !any_switched {
            final_role_runtime.insert(orig_role.handle.as_str(), orig_role.runtime);
            role_tuning_cleared.insert(orig_role.handle.as_str(), false);
            resolved_roles.push(orig_role.clone());
            continue;
        }

        // Put the first slot's chosen runtime on the created role. Clear role
        // tuning whenever the role's runtime changed OR any switched slot
        // lands on the role's runtime (so that switched slot starts with
        // defaults rather than inheriting the role's original tuning).
        let target_role_runtime = entries
            .first()
            .map(|&(_, _, chosen, _)| chosen)
            .unwrap_or(orig_role.runtime);
        let any_switched_on_role_rt = entries
            .iter()
            .any(|&(_, _, chosen, switched)| switched && chosen == target_role_runtime);
        let clear_role_tuning = target_role_runtime != orig_role.runtime || any_switched_on_role_rt;

        final_role_runtime.insert(orig_role.handle.as_str(), target_role_runtime);
        role_tuning_cleared.insert(orig_role.handle.as_str(), clear_role_tuning);
        resolved_roles.push(RoleTemplate {
            handle: orig_role.handle.clone(),
            display_name: orig_role.display_name.clone(),
            runtime: target_role_runtime,
            model: if clear_role_tuning {
                None
            } else {
                orig_role.model.clone()
            },
            effort: if clear_role_tuning {
                None
            } else {
                orig_role.effort.clone()
            },
            codex_speed: if clear_role_tuning || !supports_codex_speed(target_role_runtime) {
                None
            } else {
                orig_role.codex_speed
            },
            system_prompt: orig_role.system_prompt.clone(),
        });
    }

    let mut resolved_slots = Vec::with_capacity(template.slots.len());
    for (idx, slot) in template.slots.iter().enumerate() {
        let orig_role = orig_roles[slot.role.as_str()];
        let role_rt = final_role_runtime[slot.role.as_str()];
        let cleared_role = role_tuning_cleared[slot.role.as_str()];
        let (_, chosen, switched) = slot_choices[idx];
        let any_role_slot_switched = slots_per_role
            .get(slot.role.as_str())
            .is_some_and(|entries| entries.iter().any(|(_, _, _, sw)| *sw));

        if !any_role_slot_switched {
            resolved_slots.push(ResolvedSlot {
                slot_handle: slot.handle.clone(),
                template_role_handle: slot.role.clone(),
                runtime_override: slot.runtime,
                model_override: slot.model.clone(),
                effort_override: slot.effort.clone(),
                codex_speed_override: slot.codex_speed,
                effective_runtime: chosen,
            });
            continue;
        }

        if switched {
            resolved_slots.push(ResolvedSlot {
                slot_handle: slot.handle.clone(),
                template_role_handle: slot.role.clone(),
                runtime_override: (chosen != role_rt).then_some(chosen),
                model_override: None,
                effort_override: None,
                codex_speed_override: None,
                effective_runtime: chosen,
            });
            continue;
        }

        // Untouched slot on a role whose sibling(s) were switched: compute the
        // exact effective tuning this slot originally carried in the template.
        let orig_slot_diverged = slot.runtime.is_some_and(|rt| rt != orig_role.runtime);
        let (eff_model, eff_effort, eff_speed) = if orig_slot_diverged {
            (slot.model.clone(), slot.effort.clone(), slot.codex_speed)
        } else {
            (
                slot.model.clone().or_else(|| orig_role.model.clone()),
                slot.effort.clone().or_else(|| orig_role.effort.clone()),
                slot.codex_speed.or(orig_role.codex_speed),
            )
        };

        let runtime_override = if chosen != role_rt {
            Some(chosen)
        } else {
            slot.runtime
        };
        if chosen != role_rt || cleared_role {
            // Either the slot now diverges from the role runtime (so launch
            // will not inherit role tuning) or role tuning was cleared for a
            // switched sibling — carry the untouched slot's effective tuning
            // explicitly on the slot while preserving any explicit runtime pin.
            resolved_slots.push(ResolvedSlot {
                slot_handle: slot.handle.clone(),
                template_role_handle: slot.role.clone(),
                runtime_override,
                model_override: eff_model,
                effort_override: eff_effort,
                codex_speed_override: supports_codex_speed(chosen).then_some(eff_speed).flatten(),
                effective_runtime: chosen,
            });
        } else {
            resolved_slots.push(ResolvedSlot {
                slot_handle: slot.handle.clone(),
                template_role_handle: slot.role.clone(),
                runtime_override,
                model_override: slot.model.clone(),
                effort_override: slot.effort.clone(),
                codex_speed_override: supports_codex_speed(chosen)
                    .then_some(slot.codex_speed)
                    .flatten(),
                effective_runtime: chosen,
            });
        }
    }

    Ok(ResolvedCrewImport {
        roles: resolved_roles,
        slots: resolved_slots,
    })
}

/// Applies a standalone role runtime override (`role_handle -> target_runtime`),
/// clearing `model`, `effort`, and `codex_speed` when the runtime changes.
pub fn resolve_role_import(
    template: &StandaloneRoleTemplate,
    runtime_overrides: &BTreeMap<String, Runtime>,
) -> Result<ResolvedRoleImport, TemplateError> {
    let orig = &template.role;
    for (handle, &runtime) in runtime_overrides {
        if handle != &orig.handle {
            return Err(TemplateError::new(
                format!("Unknown role `@{handle}` in runtime_overrides."),
                None,
            ));
        }
        if !is_agent_runtime(runtime) {
            return Err(TemplateError::new(
                format!(
                    "Invalid target runtime \"{}\" for role @{handle}.",
                    runtime.key()
                ),
                None,
            ));
        }
    }

    let chosen = runtime_overrides
        .get(&orig.handle)
        .copied()
        .unwrap_or(orig.runtime);
    let switched = chosen != orig.runtime;

    Ok(ResolvedRoleImport {
        role: RoleTemplate {
            handle: orig.handle.clone(),
            display_name: orig.display_name.clone(),
            runtime: chosen,
            model: if switched { None } else { orig.model.clone() },
            effort: if switched { None } else { orig.effort.clone() },
            codex_speed: if switched || !supports_codex_speed(chosen) {
                None
            } else {
                orig.codex_speed
            },
            system_prompt: orig.system_prompt.clone(),
        },
    })
}
