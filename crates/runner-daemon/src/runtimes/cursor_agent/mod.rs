use super::helpers::*;
use super::*;
use runner_core::protocol::runtime_metadata::cursor_agent::{PERMISSIONS, SKILL_DIRS};

mod conversation;
mod models;

pub struct CursorAgent;
impl RuntimeAdapter for CursorAgent {
    fn catalog(&self) -> Option<RuntimeCatalog> {
        runner_core::protocol::runtime_metadata::cursor_agent::catalog()
    }
    fn permissions(&self) -> &'static Permissions {
        &PERMISSIONS
    }
    fn model_discovery(&self) -> Option<&'static ModelDiscoverySource> {
        Some(&DISCOVERY)
    }
    fn skills(&self) -> SkillSupport {
        SkillSupport {
            roots: SKILL_DIRS,
            ..Default::default()
        }
    }
    fn first_turn_argv(&self, body: Option<&str>) -> Vec<String> {
        positional_first_turn(body)
    }
    fn resume_plan(&self, prior_key: Option<&str>) -> ResumePlan {
        let prior_key = prior_key.filter(|key| is_uuid(key));
        let key = prior_key
            .map(str::to_owned)
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        ResumePlan {
            args: strings(&[
                if prior_key.is_some() {
                    "--resume"
                } else {
                    "--new-session-id"
                },
                &key,
            ]),
            prepend: false,
            assigned_key: Some(key),
            resuming: prior_key.is_some(),
        }
    }
    #[cfg(not(windows))]
    fn conversation_exists(&self, key: &str, ctx: &ProbeContext<'_>) -> Option<bool> {
        Some(
            conversation::store_path(ctx.cwd, ctx.role_env, key).is_some_and(|path| path.is_file()),
        )
    }
    fn status_hooks(&self) -> Option<&'static dyn StatusHooks> {
        Some(&ConversationHooks)
    }
    fn key_capture(&self) -> KeyCapture {
        KeyCapture::Hook
    }
    fn prompt_channels(&self) -> PromptChannels {
        PromptChannels {
            system_prompt: false,
            resend_persona_on_fresh: true,
        }
    }
    fn model_effort_args(&self, model: Option<&str>, _effort: Option<&str>) -> Vec<String> {
        trim_some(model)
            .map(|model| strings(&["--model", model]))
            .unwrap_or_default()
    }
    fn launch_args(&self, ctx: &LaunchContext<'_>) -> Vec<String> {
        let mut args = if ctx.mission {
            Vec::new()
        } else {
            strings(&["--trust"])
        };
        args.extend(self.model_effort_args(ctx.model, ctx.effort));
        args.extend(self.first_turn_argv(if ctx.resuming { None } else { ctx.first_turn }));
        args
    }
}

static DISCOVERY: ModelDiscoverySource = ModelDiscoverySource {
    order: 4,
    method: "models",
    query: models::query,
    config_home: conversation::config_dir,
};

struct ConversationHooks;
impl StatusHooks for ConversationHooks {
    fn supported(&self, windows: bool) -> bool {
        !windows
    }
    fn start_receiver(
        &self,
        spec: &SpawnSpec,
        _receiver: crate::session::hook_queue::HookReceiver,
    ) -> Option<Box<dyn HookWatcher>> {
        conversation::Watcher::new(spec).map(|watcher| Box::new(watcher) as Box<dyn HookWatcher>)
    }
    fn env(
        &self,
        _role_args: &[String],
        _plan: &ResumePlan,
        app_data_dir: &Path,
        spec: &SpawnSpec,
    ) -> std::collections::BTreeMap<String, String> {
        if self.supported(cfg!(windows)) {
            hook_env(app_data_dir, &spec.session_id)
        } else {
            Default::default()
        }
    }
}

#[cfg(test)]
mod tests;
