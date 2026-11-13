mod add_role;
mod controls;
mod launch;
pub(crate) mod launcher;
pub(crate) mod launcher_model;
mod modal;
mod project_field;
mod render;
mod role_setup;
mod terminal;

use self::project_field::{ProjectChoice, ProjectField};
use super::profile_page::{column_text, section_label, text_action};
use super::*;
use crate::chat_icon::runtime_mark;
use crate::surfaces::front_page::model::{RoleChoice, RuntimeChoice};
use crate::*;
use anyhow::bail;
use controls::*;
use gpui::prelude::*;
use gpui::{
    div, px, rems, svg, AnyElement, Context, Div, FontWeight, KeyDownEvent, PathPromptOptions,
    ScrollHandle, Window,
};
use render::*;
use runner_app::ui::{
    Button, ButtonVariant, Field, IconButton, Modal, ModelField, OverlayWidth, RoleAvatar,
    Scrollbar, SelectHandler, SelectLeading, SelectOption, StyledSelect, TextField,
};
use runner_core::protocol::model::CodexSpeed;
#[cfg(test)]
use runner_core::protocol::model::Role;
use runner_core::protocol::project::ProjectScope;
use runner_core::protocol::runtime::{
    filter_selectable_runtime_catalog, RuntimeCatalogEntry, RuntimeCatalogOption,
};
use std::{
    path::{Path, PathBuf},
    rc::Rc,
};

const MODAL_WIDTH: f32 = 560.;
const CARD_PADDING: f32 = 12.;
const COLUMN_GAP: f32 = 12.;
const ROLE_HINT: &str = "ui.starts_with_the_role_s_settings_change_any_of_them_for_this_chat_only";

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ChatMode {
    Role,
    Runtime,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NewTerminalTarget {
    MissionDrawer,
    Tab,
}
fn new_terminal_target(route: &AppRoute, mission_drawer_available: bool) -> NewTerminalTarget {
    if matches!(route, AppRoute::Mission(_)) && mission_drawer_available {
        NewTerminalTarget::MissionDrawer
    } else {
        NewTerminalTarget::Tab
    }
}
fn new_terminal_empty_pane(layout: &PaneLayout) -> Option<String> {
    let leaves = layout.root.leaves();
    leaves
        .iter()
        .find(|leaf| leaf.id == layout.focused_pane_id && leaf.session_id.is_none())
        .or_else(|| leaves.iter().find(|leaf| leaf.session_id.is_none()))
        .map(|leaf| leaf.id.clone())
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StartChatSelection {
    Role,
    RoleRuntime,
    Effort,
    Speed,
}

pub(crate) struct StartChatModal {
    mission_role: Option<add_role::MissionRoleForm>,
    roles: Vec<RoleChoice>,
    runtimes: Vec<RuntimeChoice>,
    role_id: Option<String>,
    role_runtime_override: Option<String>,
    effort: String,
    speed: String,
    model: Entity<TextField>,
    model_field: Entity<ModelField>,
    role_select: Entity<StyledSelect>,
    role_runtime_select: Entity<StyledSelect>,
    effort_select: Entity<StyledSelect>,
    speed_select: Entity<StyledSelect>,
    scroll_handle: ScrollHandle,
    scrollbar: Entity<Scrollbar>,
    reset_focus: [FocusHandle; 4],
    close_focus: FocusHandle,
    cancel_focus: FocusHandle,
    submit_focus: FocusHandle,
    submitting: bool,
    error: Option<crate::i18n::Text>,
    _model_subscription: gpui::Subscription,
}
impl StartChatModal {
    fn selected_role(&self) -> Option<&RoleChoice> {
        self.role_id
            .as_deref()
            .and_then(|role_id| self.roles.iter().find(|role| role.id == role_id))
    }

    fn runtime_entry(&self, name: &str) -> Option<&RuntimeChoice> {
        self.runtimes
            .iter()
            .find(|runtime| runtime.name.key() == name)
    }

    /// The agent a role chat runs on: the override, else the role's own.
    fn role_agent(&self) -> Option<&str> {
        self.role_runtime_override
            .as_deref()
            .or_else(|| self.selected_role().map(|role| role.runtime.as_str()))
    }

    /// Whether a role chat runs on the role's own agent, where the controls
    /// start from the role's model, effort and speed.
    fn on_own_agent(&self) -> bool {
        self.role_runtime_override.is_none()
    }

    fn active_runtime(&self) -> Option<&RuntimeChoice> {
        self.role_agent().and_then(|name| self.runtime_entry(name))
    }

    fn runtime_has_speed(&self, key: &str) -> bool {
        self.runtimes
            .iter()
            .find(|runtime| runtime.name.key() == key)
            .map(|runtime| runtime.speed)
            .unwrap_or_else(|| crate::runtime_ui::catalog_capabilities(&[], key).codex_speed)
    }

    fn codex_speed_visible(&self) -> bool {
        let key = self.role_agent();
        key.is_some_and(|key| self.runtime_has_speed(key))
    }

    fn effective_speed(&self) -> Option<CodexSpeed> {
        self.codex_speed_visible()
            .then(|| parse_speed(&self.speed))
            .flatten()
    }

    /// The Effort that leaves the control untouched: the role's on its own
    /// agent, else the agent's default, else none known.
    fn baseline_effort(&self) -> String {
        let runtime = self.active_runtime();
        inheriting_role(self)
            .and_then(|role| trimmed(role.effort.as_deref()))
            .map(str::to_owned)
            .or_else(|| {
                let runtime = runtime?;
                runtime
                    .default_effort
                    .clone()
                    .filter(|effort| runtime.efforts.iter().any(|option| option.value == *effort))
            })
            .unwrap_or_default()
    }

    /// The Speed that leaves the control untouched: the role's on its own
    /// Codex agent, else Inherit.
    fn baseline_speed(&self) -> String {
        match inheriting_role(self)
            .filter(|role| self.runtime_has_speed(&role.runtime))
            .and_then(|role| role.codex_speed)
        {
            Some(CodexSpeed::Standard) => "standard",
            Some(CodexSpeed::Fast) => "fast",
            None => "inherit",
        }
        .into()
    }

    /// The model the request carries: the one typed, unless it is the role's
    /// own, which leaves the control untouched.
    fn model_override(&self, cx: &App) -> Option<String> {
        let role_model = inheriting_role(self).and_then(|role| trimmed(role.model.as_deref()));
        normalized_value(self.model.read(cx).text())
            .filter(|text| Some(text.as_str()) != role_model)
    }

    fn effort_override(&self) -> Option<String> {
        (!self.effort.is_empty() && self.effort != self.baseline_effort())
            .then(|| self.effort.clone())
    }

    fn speed_override(&self) -> Option<CodexSpeed> {
        (self.speed != self.baseline_speed())
            .then(|| self.effective_speed())
            .flatten()
    }

    /// The role controls that differ from the role's own setup. On the role's
    /// own agent that is the model, effort and speed changed; on another agent
    /// it is the agent alone, whose defaults the rest start from.
    fn role_overrides(&self, cx: &App) -> RoleOverrides {
        let inheriting = inheriting_role(self).is_some();
        RoleOverrides {
            runtime: self.selected_role().is_some() && !self.on_own_agent(),
            model: inheriting && self.model_override(cx).is_some(),
            effort: inheriting && self.effort_override().is_some(),
            speed: inheriting && self.speed_override().is_some(),
        }
    }

    fn can_submit(&self) -> bool {
        !self.submitting && self.selected_role().is_some()
    }
    fn is_composing(&self, cx: &App) -> bool {
        self.model.read(cx).is_composing()
    }
}
/// Which role controls carry an override, each with its amber dot and Reset.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct RoleOverrides {
    runtime: bool,
    model: bool,
    effort: bool,
    speed: bool,
}

/// A role control's Reset, in `StartChatModal::reset_focus` order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ResetKind {
    Runtime,
    Model,
    Effort,
    Speed,
}

impl NativeRoot {
    pub(crate) fn create_modal_open(&self) -> bool {
        self.start_chat_modal.is_some()
    }
    fn confirm_start_chat(
        &mut self,
        _: &ConfirmStartChat,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(modal) = &self.start_chat_modal {
            for select in [
                &modal.role_select,
                &modal.role_runtime_select,
                &modal.effort_select,
                &modal.speed_select,
            ] {
                select.update(cx, |select, cx| select.close(cx));
            }
            modal.model_field.update(cx, |field, cx| field.close(cx));
        }
        self.submit_mission_role(window, cx);
    }
}
impl NativeRoot {
    /// The cwd a shell started from `session_id` opens in: see
    /// `shell_start_cwd`. Only shell sessions scan for OSC 7, so a chat's
    /// cwd never moves.
    pub(crate) fn session_start_cwd(&self, session_id: &str, cx: &App) -> Option<String> {
        let live = self
            .app_store
            .read(cx)
            .bridge
            .session(session_id)
            .and_then(|terminal| terminal.live_cwd());
        shell_start_cwd(
            live,
            self.session_entry(session_id, cx)
                .and_then(|entry| entry.cwd.as_deref()),
        )
    }
}

/// Where a new shell opens when it is started from another session (#575):
/// the directory that session's shell last reported through OSC 7, which
/// `live_cwd` only offers while it exists, else the cwd it was spawned in.
/// `None` continues down `terminal_working_dir`'s chain.
pub(crate) fn shell_start_cwd(live: Option<PathBuf>, spawn: Option<&str>) -> Option<String> {
    live.and_then(|cwd| cwd.into_os_string().into_string().ok())
        .or_else(|| spawn.map(str::to_owned))
}

pub(crate) fn terminal_working_dir(
    sibling_cwd: Option<&str>,
    project_cwd: Option<&str>,
    default_cwd: &str,
    home: Option<&str>,
) -> Option<String> {
    [sibling_cwd, project_cwd, Some(default_cwd), home]
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|path| !path.is_empty())
        .map(str::to_owned)
}

pub(crate) fn load_selectable_runtimes(
    core: &DaemonClient,
    settings: &AppSettings,
) -> (Vec<RuntimeCatalogEntry>, bool, Option<String>) {
    let checking = core.discovery_snapshot().is_ok_and(|state| state.checking);
    match core.runtime_catalog() {
        Ok(catalog) => {
            let enabled = catalog
                .iter()
                .filter(|runtime| settings.is_agent_enabled(runtime.name, runtime.default_enabled))
                .map(|runtime| runtime.name.to_string())
                .collect::<Vec<_>>();
            (
                filter_selectable_runtime_catalog(catalog, Some(&enabled)),
                checking,
                None,
            )
        }
        Err(error) => (Vec::new(), checking, Some(error.to_string())),
    }
}

fn runtime_display_name(runtimes: &[RuntimeChoice], name: &str) -> String {
    runtimes
        .iter()
        .find(|runtime| runtime.name.key() == name)
        .map(|runtime| runtime.display_name.clone())
        .or_else(|| {
            runner_core::protocol::runtime_metadata::runtime_list()
                .into_iter()
                .find(|runtime| runtime.name.key() == name)
                .map(|runtime| runtime.display_name)
        })
        .unwrap_or_else(|| name.to_owned())
}

fn normalized_value(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use gpui::{size, Render, TestAppContext, VisualTestContext, WindowHandle};
    use runner_core::protocol::model::Runtime;
    use runner_daemon::{db, session, shell_path};
    use std::{fs, sync::RwLock};
    struct ModalHost(Entity<NativeRoot>);
    impl Render for ModalHost {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            self.0.update(cx, |root, cx| {
                if root
                    .start_chat_modal
                    .as_ref()
                    .is_some_and(|modal| modal.mission_role.is_none())
                {
                    return div().into_any_element();
                }
                root.render_app_shell(window, cx)
            })
        }
    }
    pub(in crate::surfaces) struct ModalHarness {
        pub(in crate::surfaces) visual: VisualTestContext,
        host: WindowHandle<ModalHost>,
        _cx: TestAppContext,
        _temp: tempfile::TempDir,
        _theme: crate::theme_snapshot::ThemeGuard,
    }

    pub(in crate::surfaces) fn modal_harness(
        width: f32,
        height: f32,
        roles: Vec<Role>,
        runtimes: Vec<RuntimeCatalogEntry>,
        mode: ChatMode,
    ) -> ModalHarness {
        let theme = crate::theme_snapshot::ThemeGuard::new();
        let temp = tempfile::tempdir().unwrap();
        let runtime_shell_env = Arc::new(RwLock::new(shell_path::LoginShellEnv::default()));
        let runtime_discovery =
            Arc::new(RwLock::new(shell_path::DiscoveryState::startup(None, None)));
        let core = crate::test_support::core(
            Arc::new(db::open_fixture_pool(&temp.path().join("runner.db")).unwrap()),
            temp.path().to_owned(),
            session::SessionManager::new(
                runtime_shell_env.clone(),
                runtime_discovery.clone(),
                Arc::new(session::pty_runtime::PtyRuntime::new()),
            ),
            runtime_shell_env,
            runtime_discovery,
        );
        let mut cx = TestAppContext::single();
        cx.update(|cx| {
            keymap::install_bindings(
                cx,
                &keymap::KeymapOverrides::new(),
                crate::keymap::TerminalCtrlKeys::RunnerFirst,
                false,
            )
        });
        #[cfg(target_os = "macos")]
        let updater = cx.new(|cx| Updater::new(false, cx));
        #[cfg(any(windows, target_os = "linux"))]
        let updater = cx.new(|cx| Updater::new(false, temp.path().join("updates"), cx));
        cx.set_global(GlobalUpdater(updater));
        let store = cx.new(|cx| {
            AppStore::new(
                core,
                None,
                None,
                temp.path().join("settings.json"),
                AppSettings::default(),
                None,
                cx,
            )
        });
        cx.set_global(GlobalAppStore(store.clone()));
        cx.set_global(WindowLayoutCheckpoint::default());
        let host = cx.add_window(|window, cx| {
            let root = cx.new(|cx| {
                NativeRoot::new(
                    "test-chat-modal".into(),
                    temp.path().to_owned(),
                    Some("chats".into()),
                    None,
                    store,
                    window,
                    cx,
                )
            });
            root.update(cx, |root, cx| {
                root.root_focus.focus(window, cx);
                root.build_add_role_form(
                    roles.into_iter().map(Into::into).collect(),
                    runtimes.into_iter().map(Into::into).collect(),
                    window,
                    cx,
                );
                if mode == ChatMode::Role {
                    let form = add_role::MissionRoleForm::fixture(cx);
                    root.start_chat_modal.as_mut().unwrap().mission_role = Some(form);
                }
            });
            ModalHost(root)
        });
        cx.run_until_parked();
        let visual = VisualTestContext::from_window(host.into(), &cx);
        visual.simulate_resize(size(px(width), px(height)));
        visual.run_until_parked();
        ModalHarness {
            visual,
            host,
            _cx: cx,
            _temp: temp,
            _theme: theme,
        }
    }

    impl ModalHarness {
        pub(in crate::surfaces) fn act(
            &mut self,
            f: impl FnOnce(&mut NativeRoot, &mut Window, &mut Context<NativeRoot>),
        ) {
            self.host
                .update(&mut self.visual, |host, window, cx| {
                    host.0.update(cx, |root, cx| f(root, window, cx));
                    cx.notify();
                })
                .unwrap();
            self.visual.run_until_parked();
        }
    }

    impl ModalHarness {
        fn edit(&mut self, f: impl FnOnce(&mut StartChatModal, &mut Context<NativeRoot>)) {
            self.act(|root, _, cx| {
                f(root.start_chat_modal.as_mut().unwrap(), cx);
                cx.notify();
            });
        }

        fn read<R>(&self, f: impl FnOnce(&StartChatModal, &App) -> R) -> R {
            self.host
                .read_with(&self.visual, |host, cx| {
                    f(host.0.read(cx).start_chat_modal.as_ref().unwrap(), cx)
                })
                .unwrap()
        }

        pub(in crate::surfaces) fn read_root<R>(
            &self,
            f: impl FnOnce(&NativeRoot, &App) -> R,
        ) -> R {
            self.host
                .read_with(&self.visual, |host, cx| f(host.0.read(cx), cx))
                .unwrap()
        }

        pub(in crate::surfaces) fn bounds(
            &mut self,
            selector: &str,
        ) -> Option<gpui::Bounds<gpui::Pixels>> {
            self.visual
                .debug_bounds(Box::leak(selector.to_owned().into_boxed_str()))
        }

        fn choose(&mut self, selection: StartChatSelection, value: &str) {
            let value = value.to_owned();
            self.act(move |root, _, cx| {
                let modal = root.start_chat_modal.as_ref().unwrap();
                let select = match selection {
                    StartChatSelection::Role => modal.role_select.clone(),
                    StartChatSelection::RoleRuntime => modal.role_runtime_select.clone(),
                    StartChatSelection::Effort => modal.effort_select.clone(),
                    StartChatSelection::Speed => modal.speed_select.clone(),
                };
                select.update(cx, |select, cx| select.set_value(value.clone(), cx));
                root.select_start_chat_choice(selection, &value, cx);
            });
        }

        fn type_model(&mut self, text: &'static str) {
            self.edit(|modal, cx| modal.model.update(cx, |input, cx| input.set_text(text, cx)));
        }

        fn reset(&mut self, kind: ResetKind) {
            let bounds = self
                .bounds(&format!("START_CHAT_RESET {kind:?}"))
                .expect("the control shows its Reset");
            let content = self.bounds("MODAL_CONTENT").expect("modal content");
            assert!(content.contains(&bounds.origin) && content.contains(&bounds.bottom_right()));
            self.visual
                .simulate_click(bounds.center(), gpui::Modifiers::default());
            self.visual.run_until_parked();
        }

        fn effort_choice(&self) -> (String, bool) {
            self.read(|modal, cx| {
                let option = modal.effort_select.read(cx).selected().unwrap();
                (option.label.to_string(), option.marked)
            })
        }

        fn overrides(&self) -> RoleOverrides {
            self.read(|modal, cx| modal.role_overrides(cx))
        }
    }
    pub(in crate::surfaces) fn project_fixture(
        harness: &mut ModalHarness,
    ) -> Vec<runner_core::protocol::project::ProjectRow> {
        let base = harness._temp.path().join("projects");
        let mut projects = Vec::new();
        harness.act(|root, _, cx| {
            for name in ["alpha", "beta"] {
                let path = base.join(name);
                fs::create_dir_all(path.join(".worktrees/feature")).unwrap();
                projects.push(
                    root.core(cx)
                        .project_create(name.into(), path.to_string_lossy().into_owned())
                        .unwrap(),
                );
            }
            root.app_store.update(cx, |store, cx| {
                store.projects = projects.clone();
                cx.notify();
            });
        });
        projects
    }
    pub(super) fn test_role(handle: &str, runtime: &str) -> Role {
        Role {
            id: format!("role-{handle}"),
            handle: handle.into(),
            display_name: format!("Role {handle}"),
            runtime: runtime.into(),
            command: runtime.into(),
            args: Vec::new(),
            working_dir: None,
            system_prompt: None,
            env: Default::default(),
            model: None,
            effort: None,
            codex_speed: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    fn agents() -> Vec<RuntimeCatalogEntry> {
        let mut claude = runtime("claude-code", &["", "low", "high", "xhigh"]);
        claude.display_name = "Claude Code".into();
        claude.command = "claude".into();
        claude.default_effort = Some("high".into());
        let mut codex = runtime("codex", &["", "low", "medium"]);
        codex.display_name = "Codex".into();
        codex.default_model = Some("gpt-5.5".into());
        codex.default_effort = Some("medium".into());
        vec![claude, codex]
    }

    fn claude_role() -> Role {
        Role {
            model: Some("opus[1m]".into()),
            effort: Some("xhigh".into()),
            ..test_role("impl-claude", "claude-code")
        }
    }

    fn codex_role() -> Role {
        Role {
            codex_speed: Some(CodexSpeed::Fast),
            ..test_role("impl-codex", "codex")
        }
    }

    fn runtime(name: &str, efforts: &[&str]) -> RuntimeCatalogEntry {
        RuntimeCatalogEntry {
            installed_version: None,
            available_version: None,
            min_version: None,
            below_minimum: false,
            name: Runtime::parse(name).unwrap(),
            capabilities: runner_core::protocol::runtime::RuntimeCatalogEntry::for_runtime(
                Runtime::parse(name).unwrap(),
            )
            .map(|entry| entry.capabilities)
            .unwrap_or_default(),
            display_name: name.into(),
            command: name.into(),
            native_fork: matches!(name, "codex" | "claude-code" | "pi"),
            description: name.into(),
            install_url: String::new(),
            default_enabled: true,
            available: true,
            default_model: None,
            default_effort: None,
            models: Vec::new(),
            efforts: efforts
                .iter()
                .map(|value| RuntimeCatalogOption {
                    value: (*value).into(),
                    label: (*value).into(),
                    description: None,
                    supported_efforts: None,
                })
                .collect(),
        }
    }

    #[test]
    fn add_role_shows_inherited_values_and_sends_no_override() {
        let mut modal = modal_harness(1200., 1000., vec![claude_role()], agents(), ChatMode::Role);
        assert!(modal.bounds("START_CHAT_SETUP").is_some());
        assert_eq!(modal.overrides(), RoleOverrides::default());
        modal.read(|modal, cx| {
            assert_eq!(modal.model_override(cx), None);
            assert_eq!(modal.effort_override(), None);
            assert_eq!(modal.speed_override(), None);
            assert_eq!(modal.role_runtime_override, None);
            assert_eq!(modal.effort, "xhigh");
            assert_eq!(
                model_placeholder(inheriting_role(modal)).resolve(),
                "opus[1m]"
            );
            assert!(modal.model.read(cx).placeholder_uses_value_style());
        });
        assert_eq!(modal.effort_choice(), ("xhigh".into(), false));

        // A control still showing the role's value is untouched, however it got there.
        modal.type_model("opus[1m]");
        modal.choose(StartChatSelection::Effort, "low");
        modal.choose(StartChatSelection::Effort, "xhigh");
        assert_eq!(modal.overrides(), RoleOverrides::default());
        modal.read(|modal, cx| {
            assert_eq!(modal.model_override(cx), None);
            assert_eq!(modal.effort_override(), None);
        });
    }

    #[test]
    fn a_changed_value_carries_the_dot_and_its_reset_restores_only_that_value() {
        let mut modal = modal_harness(1200., 1000., vec![claude_role()], agents(), ChatMode::Role);
        modal.type_model("sonnet");
        modal.choose(StartChatSelection::Effort, "low");
        assert_eq!(
            modal.overrides(),
            RoleOverrides {
                runtime: false,
                model: true,
                effort: true,
                speed: false
            }
        );
        assert!(modal.bounds("MODEL_FIELD_MARK").is_some());
        assert_eq!(modal.effort_choice(), ("low".into(), true));
        assert!(modal
            .bounds("START_CHAT_NOTE Model role: opus[1m]")
            .is_some());
        assert!(modal.bounds("START_CHAT_NOTE Effort role: xhigh").is_some());

        modal.reset(ResetKind::Model);
        modal.read(|modal, cx| {
            assert_eq!(modal.model.read(cx).text(), "");
            assert_eq!(modal.effort, "low", "Reset restores one value only");
        });
        assert_eq!(
            modal.overrides(),
            RoleOverrides {
                effort: true,
                ..RoleOverrides::default()
            }
        );

        modal.reset(ResetKind::Effort);
        assert_eq!(modal.read(|modal, _| modal.effort.clone()), "xhigh");
        assert_eq!(modal.effort_choice(), ("xhigh".into(), false));
        assert_eq!(modal.overrides(), RoleOverrides::default());
    }

    #[test]
    fn choosing_the_roles_own_agent_is_not_an_override() {
        let mut modal = modal_harness(1200., 1000., vec![claude_role()], agents(), ChatMode::Role);
        modal.choose(StartChatSelection::RoleRuntime, "codex");
        assert_eq!(
            modal.read(|modal, _| modal.role_runtime_override.clone()),
            Some("codex".into())
        );
        modal.choose(StartChatSelection::RoleRuntime, "claude-code");
        modal.read(|modal, cx| {
            assert_eq!(modal.role_runtime_override, None);
            assert_eq!(modal.role_runtime_select.read(cx).value(), "claude-code");
        });
        assert_eq!(modal.overrides(), RoleOverrides::default());
    }

    #[test]
    fn another_role_starts_from_its_own_setup() {
        let other = Role {
            model: Some("gpt-5.5".into()),
            ..test_role("other", "codex")
        };
        let mut modal = modal_harness(
            1200.,
            1000.,
            vec![claude_role(), other.clone()],
            agents(),
            ChatMode::Role,
        );
        modal.choose(StartChatSelection::RoleRuntime, "codex");
        modal.type_model("gpt-5.6-sol");
        modal.choose(StartChatSelection::Role, &other.id);
        modal.read(|modal, cx| {
            assert_eq!(modal.role_runtime_override, None);
            assert_eq!(modal.model.read(cx).text(), "");
            assert_eq!(modal.effort, "medium", "the codex agent's own default");
        });
        assert_eq!(modal.overrides(), RoleOverrides::default());
    }

    #[test]
    fn another_agent_shows_its_own_defaults_at_full_contrast_and_only_runtime_carries_the_dot() {
        let mut modal = modal_harness(1200., 1000., vec![claude_role()], agents(), ChatMode::Role);
        modal.choose(StartChatSelection::RoleRuntime, "codex");
        assert_eq!(
            modal.overrides(),
            RoleOverrides {
                runtime: true,
                ..RoleOverrides::default()
            }
        );
        assert!(modal.read(|modal, cx| {
            modal
                .role_runtime_select
                .read(cx)
                .selected()
                .unwrap()
                .marked
        }));
        assert!(modal
            .bounds("START_CHAT_NOTE Runtime role: Claude Code")
            .is_some());
        assert!(modal.bounds("START_CHAT_RESET Runtime").is_some());
        modal.read(|modal, cx| {
            assert!(inheriting_role(modal).is_none());
            // The agent's defaults, not the role's model and effort. The
            // model stays generic because no concrete value reaches launch.
            assert_eq!(
                model_placeholder(inheriting_role(modal)).resolve(),
                "default"
            );
            assert_eq!(modal.effort, "medium");
            assert_eq!(modal.model_override(cx), None);
            assert_eq!(modal.effort_override(), None);
        });
        assert_eq!(modal.effort_choice(), ("medium".into(), false));

        // Changing the agent's model is not the role's to override: no dot.
        modal.type_model("gpt-5.6-sol");
        assert!(modal.bounds("MODEL_FIELD_MARK").is_none());
        modal.read(|modal, cx| {
            assert_eq!(modal.model_override(cx), Some("gpt-5.6-sol".into()));
        });

        modal.reset(ResetKind::Runtime);
        modal.read(|modal, cx| {
            assert_eq!(modal.role_runtime_override, None);
            assert_eq!(modal.model.read(cx).text(), "");
            assert_eq!(modal.effort, "xhigh");
            assert_eq!(modal.role_runtime_select.read(cx).value(), "claude-code");
        });
        assert_eq!(modal.overrides(), RoleOverrides::default());
    }

    #[test]
    fn a_role_without_a_model_or_effort_starts_from_its_agents_defaults() {
        let modal = modal_harness(
            1200.,
            1000.,
            vec![test_role("plain", "claude-code")],
            agents(),
            ChatMode::Role,
        );
        modal.read(|modal, cx| {
            assert_eq!(
                model_placeholder(inheriting_role(modal)).resolve(),
                "default",
                "the role does not pin a model"
            );
            assert_eq!(modal.effort, "high", "the agent's default effort");
            assert_eq!(modal.effort_override(), None);
            assert_eq!(modal.model_override(cx), None);
            assert!(modal.model.read(cx).placeholder_uses_value_style());
        });
        assert_eq!(modal.effort_choice(), ("high".into(), false));
        drop(modal);

        // No default effort known: the control reads `default`, never a blank.
        let mut claude = agents().remove(0);
        claude.default_effort = None;
        let modal = modal_harness(
            1200.,
            1000.,
            vec![test_role("plain", "claude-code")],
            vec![claude],
            ChatMode::Role,
        );
        assert_eq!(modal.effort_choice(), ("default".into(), false));
        modal.read(|modal, _| assert_eq!(modal.effort, ""));
    }

    #[test]
    fn speed_options_mark_values_that_override_the_role() {
        let values = |options: Vec<SelectOption>| {
            options
                .iter()
                .map(|option| (option.value.clone(), option.marked))
                .collect::<Vec<_>>()
        };
        // A role that sets a speed cannot be told to inherit: no such choice.
        assert_eq!(
            values(speed_options(Some("fast"), true)),
            [("standard".to_owned(), true), ("fast".to_owned(), false)]
        );
        assert_eq!(
            values(speed_options(Some("inherit"), false)),
            [
                ("inherit".to_owned(), false),
                ("standard".to_owned(), true),
                ("fast".to_owned(), true)
            ]
        );
        assert_eq!(
            values(speed_options(None, false)),
            [
                ("inherit".to_owned(), false),
                ("standard".to_owned(), false),
                ("fast".to_owned(), false)
            ]
        );
    }

    #[test]
    fn role_speed_remains_visible_when_its_agent_is_not_selectable() {
        let mut modal = modal_harness(
            1200.,
            1000.,
            vec![codex_role()],
            vec![agents().remove(0)],
            ChatMode::Role,
        );
        assert!(modal.read(|modal, _| modal.codex_speed_visible()));
        assert!(modal.bounds("START_CHAT_SPEED_FIELD").is_some());
        modal.read(|modal, _| assert_eq!(modal.baseline_speed(), "fast"));
    }

    #[test]
    fn reselecting_the_current_agent_preserves_add_role_overrides() {
        let mut modal = modal_harness(1200., 1000., vec![claude_role()], agents(), ChatMode::Role);
        for runtime in ["claude-code", "codex"] {
            modal.choose(StartChatSelection::RoleRuntime, runtime);
            modal.type_model("custom-model");
            modal.choose(StartChatSelection::Effort, "low");
            modal.choose(StartChatSelection::Speed, "fast");
            modal.choose(StartChatSelection::RoleRuntime, runtime);
            modal.read(|modal, cx| {
                assert_eq!(modal.model.read(cx).text(), "custom-model");
                assert_eq!(modal.effort, "low");
                assert_eq!(modal.speed, "fast");
            });
        }
    }

    #[test]
    fn antigravity_effort_follows_model_support_and_disables_for_leveled_or_blank_model() {
        let mut antigravity =
            runner_core::protocol::runtime::RuntimeCatalogEntry::for_runtime(Runtime::Antigravity)
                .unwrap();
        antigravity.available = true;
        let mut modal = modal_harness(
            1200.,
            1000.,
            vec![test_role("plain-agy", "antigravity")],
            vec![antigravity],
            ChatMode::Role,
        );
        let effort_disabled = |modal: &ModalHarness| {
            modal.read(|modal, cx| modal.effort_select.read(cx).is_disabled())
        };
        // Blank role model with no launch model disables Effort.
        assert_eq!(modal.effort_choice(), ("default".into(), false));
        assert!(effort_disabled(&modal));

        // Base model enables only its supported effort levels.
        modal.type_model("gemini-4-hydrogen");
        assert_eq!(modal.effort_choice(), ("default".into(), false));
        assert!(!effort_disabled(&modal));
        modal.choose(StartChatSelection::Effort, "xhigh");
        assert_eq!(modal.effort_choice(), ("xhigh".into(), true));
        assert!(!effort_disabled(&modal));

        // Switching to a model that does not support xhigh resets effort cleanly.
        modal.type_model("gemini-4-carbon");
        assert_eq!(modal.effort_choice(), ("default".into(), false));
        assert!(!effort_disabled(&modal));

        // Models without efforts or leveled IDs disable Effort without offering conflicting choices.
        for model in [
            "gemini-pro-orchestrator",
            "gemini-4-hydrogen-xhigh",
            "gpt-oss-120b-medium",
        ] {
            modal.type_model(model);
            assert_eq!(modal.effort_choice(), ("default".into(), false), "{model}");
            assert!(effort_disabled(&modal), "{model}");
        }
    }
}
