use super::*;

const LOCALE_VARS: [&str; 3] = ["LANG", "LC_ALL", "LC_CTYPE"];
use runner_core::protocol::{SESSION_CLI_ENV, SESSION_ID_ENV};

/// Dock-launched GUI apps inherit no locale, so children run in the
/// POSIX C locale and macOS clipboard tools (claude's `pbpaste`)
/// decode UTF-8 as Mac Roman — CJK text arrives as mojibake (#461).
/// Mirror alacritty's minimal macOS fallback: set `LC_CTYPE=UTF-8`
/// only when no layer supplies a locale (a full `LANG` would also
/// switch tool message languages). Children inherit the app's own
/// process env on top of the spec, so a locale there counts too.
pub(super) fn ensure_utf8_locale(env: &mut BTreeMap<String, String>, process_has_locale: bool) {
    if process_has_locale || LOCALE_VARS.iter().any(|var| env.contains_key(*var)) {
        return;
    }
    env.insert("LC_CTYPE".into(), "UTF-8".into());
}

/// The environment an agent process starts from. Login-shell vars (the
/// captured configuration and proxy variables) are the bottom layer; a role row can override
/// any of them, being the most specific configuration surface. System vars
/// layer on top so the user can't accidentally shadow them — PATH comes from
/// the composed path and a role.env PATH is filtered by
/// `launch::is_reserved_env_name` — then `extra_env`, the runtime's quiet
/// flags (#475), and a UTF-8 locale when no layer supplies one.
pub(super) fn agent_env(
    shell_vars: BTreeMap<String, String>,
    role_env: &HashMap<String, String>,
    extra_env: BTreeMap<String, String>,
    runtime_key: &str,
) -> BTreeMap<String, String> {
    let mut env = shell_vars;
    for (k, v) in role_env {
        env.insert(k.clone(), v.clone());
    }
    env.insert("TERM".into(), "xterm-256color".into());
    env.insert("COLORTERM".into(), "truecolor".into());
    if extra_env.contains_key("RUNNER_MISSION_ID") && !extra_env.contains_key("RUNNER_CREW_ID") {
        env.remove("RUNNER_CREW_ID");
    }
    env.extend(extra_env);
    for &(name, value) in crate::runtimes::for_key(runtime_key).launch_env() {
        env.insert(name.into(), value.into());
    }
    let process_has_locale = LOCALE_VARS
        .iter()
        .any(|var| std::env::var_os(var).is_some());
    ensure_utf8_locale(&mut env, process_has_locale);
    env
}

impl SessionManager {
    pub(super) fn spawn_runtime(
        &self,
        mut spec: SpawnSpec,
    ) -> super::super::runtime::RuntimeResult<(RuntimeSession, OutputStream)> {
        if spec
            .env
            .contains_key(runner_core::protocol::hook::GENERATION_ENV)
        {
            if let Some(endpoint) = self.hook_endpoint.read().unwrap().as_ref() {
                spec.env.insert(
                    runner_core::protocol::hook::ENDPOINT_ENV.into(),
                    endpoint.0.to_string_lossy().into_owned(),
                );
            }
        }
        self.runtime.spawn(spec)
    }

    pub(super) fn resolve_role_executable(&self, role: &Role, pool: &DbPool) -> Result<Role> {
        let Some(definition) = crate::runtimes::for_key(&role.runtime).catalog() else {
            return Ok(role.clone());
        };
        let mut resolved = role.clone();
        if role.command == definition.command {
            resolved.command = crate::runtime_status::effective_runtime_command(
                definition.name,
                pool,
                &self.shell_env,
                &self.discovery_state,
            )?
            .command;
        }
        let discovery = self
            .discovery_state
            .read()
            .map_err(|_| Error::msg("runtime discovery lock poisoned"))?;
        let installed = discovery
            .versions
            .installed(definition.name, &resolved.command);
        if crate::runtime_status::versions::below_minimum(installed, definition.min_version) {
            return Err(runner_core::protocol::runtime::RuntimeVersionGate {
                runtime: definition.name,
                installed_version: installed.unwrap().into(),
                min_version: definition.min_version.unwrap().into(),
            }
            .into());
        }
        Ok(resolved)
    }

    pub(super) fn enter_launch_gate(&self, session_id: &str, runtime_key: &str) {
        crate::runtimes::for_key(runtime_key).launch_gate(session_id, &self.launch_gate);
    }

    pub(super) fn seed_runtime_project_trust(
        &self,
        session_id: &str,
        runtime_key: &str,
        cwd: Option<&Path>,
        copilot_home: Option<&str>,
        claude_config_dir: Option<&str>,
    ) {
        let result = crate::runtimes::for_key(runtime_key).seed_trust(
            session_id,
            cwd,
            copilot_home,
            claude_config_dir,
        );
        if let (Err(e), Some(cwd)) = (result, cwd) {
            log::warn!(
                "failed to seed {:?} project trust: session={session_id} cwd={} error={e}",
                Runtime::parse(runtime_key),
                cwd.display()
            );
        }
    }

    /// Build a `SpawnSpec` skeleton with the manager's stable inputs
    /// (shell PATH, role env after merging system vars). The
    /// runtime adapter argv (resume_plan + trailing_runtime_args)
    /// lives at the call site since it depends on a pre-resolved
    /// `agent_session_key`.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn base_spawn_spec(
        &self,
        session_id: String,
        role: &Role,
        cwd: Option<String>,
        mission: bool,
        shim_dir: Option<PathBuf>,
        bundled_bin_dir: Option<PathBuf>,
        initial_size: Option<(u16, u16)>,
        extra_env: BTreeMap<String, String>,
    ) -> SpawnSpec {
        let shell_env = self
            .shell_env
            .read()
            .expect("runtime shell environment lock poisoned")
            .clone();
        let mut env = agent_env(shell_env.vars, &role.env, extra_env, &role.runtime);
        // Every agent session can address itself, so `runner mission start`
        // from a chat links the mission to it (#748). Terminals cannot be linked.
        if !Runtime::parse(&role.runtime).is_some_and(Runtime::is_shell) {
            env.insert(SESSION_ID_ENV.into(), session_id.clone());
        }
        // Login shells may put another build's `runner` ahead of these
        // directories; that `runner` hands the invocation back to this one (#1149).
        if let Some(cli) = shim_dir.as_ref().or(bundled_bin_dir.as_ref()) {
            env.insert(
                SESSION_CLI_ENV.into(),
                cli.join(runner_core::cli_install::AGENT_DEST_BIN_NAME)
                    .to_string_lossy()
                    .into_owned(),
            );
        }
        SpawnSpec {
            agent_runtime: Runtime::parse(&role.runtime),
            pending_turn: None,
            session_id,
            cwd: cwd.map(PathBuf::from),
            command: role.command.clone(),
            args: role.args.clone(),
            env,
            mission,
            shim_dir,
            bundled_bin_dir,
            shell_path: shell_env.path,
            initial_size,
        }
    }

    /// The PTY for a CLI's own update command (#533): the agent
    /// environment `base_spawn_spec` builds, without Runner's layers — no
    /// role env, mission variables, bundled CLI, hooks or #475 quiet flags.
    /// It has no `sessions` row; `spawn_unlisted` runs it.
    pub fn update_spawn_spec(
        &self,
        command: String,
        args: Vec<String>,
        cwd: Option<PathBuf>,
        initial_size: (u16, u16),
    ) -> SpawnSpec {
        let shell_env = self
            .shell_env
            .read()
            .expect("runtime shell environment lock poisoned")
            .clone();
        SpawnSpec {
            agent_runtime: None,
            pending_turn: None,
            session_id: ulid::Ulid::new().to_string(),
            cwd,
            command,
            args,
            env: agent_env(shell_env.vars, &HashMap::new(), BTreeMap::new(), ""),
            mission: false,
            shim_dir: None,
            bundled_bin_dir: None,
            shell_path: shell_env.path,
            initial_size: Some(initial_size),
        }
    }

    /// Apply the runtime adapter's resume + trailing args to a
    /// `SpawnSpec`. Mirrors what the portable-pty `spawn` paths
    /// did inline; factored out so spawn / spawn_direct / resume
    /// can share the argv composition.
    ///
    /// `first_turn` is the composed first-user-turn body (member prompt for missions,
    /// persona for direct chats). When the runtime accepts the
    /// positional `[PROMPT]` argv and the body fits in
    /// `FIRST_TURN_ARGV_MAX_BYTES`, the body lands as the trailing
    /// positional. Returns whether the body was delivered via argv
    /// so the caller can warn if a supported runtime somehow missed
    /// the deterministic path.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn apply_runtime_args(
        &self,
        spec: &mut SpawnSpec,
        role: &Role,
        plan: &router::runtime::ResumePlan,
        app_data_dir: &Path,
        system_prompt_path: Option<&Path>,
        first_turn: Option<&str>,
        mission_bus_dir: Option<&Path>,
        pool: Option<&DbPool>,
    ) -> bool {
        let options = pool
            .and_then(|pool| crate::db::runtime_launch_options(pool).ok())
            .and_then(|options| options.get(&role.runtime).copied())
            .unwrap_or_default();
        let effective_role = crate::runtimes::apply_launch_options(role, spec.mission, options);
        let role = &effective_role;
        spec.args.clone_from(&role.args);
        if !spec.mission && !options.status_hooks {
            spec.env
                .insert(crate::runtimes::STATUS_HOOKS_ENV.into(), "false".into());
        }
        if !options.remote_plugin_catalog {
            spec.env
                .insert(crate::runtimes::REMOTE_PLUGIN_ENV.into(), "false".into());
        }
        let adapter = crate::runtimes::for_key(&role.runtime);
        let env = crate::shell_path::LoginShellEnv {
            path: spec.shell_path.clone(),
            vars: spec.env.clone(),
        };
        let default_model = Runtime::parse(&role.runtime).and_then(|runtime| {
            self.discovery_state
                .read()
                .ok()?
                .models
                .catalog(
                    runtime,
                    &crate::runtime_status::models::source_with_main(
                        runtime,
                        &spec.command,
                        &env,
                        pool,
                    ),
                )?
                .default_model
                .clone()
        });
        spec.agent_runtime = Runtime::parse(&role.runtime);
        spec.pending_turn = adapter
            .status_hooks()
            .is_some_and(|hooks| hooks.tracks_pending_turn())
            .then_some(!plan.resuming && first_turn.is_some_and(|body| !body.trim().is_empty()));
        #[cfg(windows)]
        let first_turn = if crate::session::launch::is_windows_batch(&role.command) {
            None
        } else {
            first_turn
        };
        adapter.prepare_capture(app_data_dir, &spec.session_id);
        if let Some(hooks) = adapter.status_hooks().filter(|_| {
            options.status_hooks
                || spec.mission
                || matches!(
                    Runtime::parse(&role.runtime),
                    Some(
                        Runtime::Codex
                            | Runtime::ClaudeCode
                            | Runtime::Pi
                            | Runtime::Cursor
                            | Runtime::OpenCode
                            | Runtime::Copilot
                            | Runtime::Grok
                    )
                )
        }) {
            let env = hooks.env(&role.args, plan, app_data_dir, spec);
            spec.env.extend(env);
        }
        spec.env
            .extend(adapter.launch_context_env(&crate::runtimes::LaunchContext {
                default_model: default_model.as_deref(),
                env: &spec.env,
                cwd: spec.cwd.as_deref(),
                role_args: &role.args,
                app_data_dir,
                session_id: &spec.session_id,
                resuming: plan.resuming,
                mission: spec.mission,
                model: role.model.as_deref(),
                effort: role.effort.as_deref(),
                codex_speed: role.codex_speed,
                system_prompt: None,
                first_turn,
            }));
        let (args, delivered_via_argv) = compose_args(
            adapter,
            &spec.args,
            plan,
            mission_bus_dir,
            &crate::runtimes::LaunchContext {
                default_model: default_model.as_deref(),
                env: &spec.env,
                cwd: spec.cwd.as_deref(),
                role_args: &role.args,
                app_data_dir,
                session_id: &spec.session_id,
                resuming: plan.resuming,
                model: role.model.as_deref(),
                effort: role.effort.as_deref(),
                codex_speed: role.codex_speed,
                system_prompt: system_prompt_path
                    .map(|path| path.to_string_lossy())
                    .as_deref(),
                first_turn,
                mission: spec.mission,
            },
        );
        spec.args = args;
        if is_terminal_shell(role) && !spec.mission {
            crate::shell_integration::inject(spec, app_data_dir, |name| std::env::var(name).ok());
        }
        delivered_via_argv
    }

    /// True iff the `sessions` row for `session_id` is still in the
    /// DB. False if the row was deleted out from under an in-flight
    /// spawn — most commonly `role_delete` removing the role's
    /// sessions, but also covers manual
    /// DB cleanup or any other path that drops the row while a
    /// gated spawn was asleep. Returns false on pool errors so the
    /// caller treats "can't tell" the same as "deleted" and bails
    /// out of the spawn — losing a session on a transient DB hiccup
    /// is preferable to leaving an orphan PTY attached to no row.
    pub(super) fn session_row_exists(pool: &DbPool, session_id: &str) -> bool {
        let Ok(conn) = pool.get() else { return false };
        crate::repo::session::exists(&conn, session_id).unwrap_or(false)
    }

    /// The row's persisted last size: the hint from registration until a
    /// frontend push lands, then whatever `resize` recorded.
    pub(super) fn persisted_size(pool: &DbPool, session_id: &str) -> Option<(u16, u16)> {
        let conn = pool.get().ok()?;
        let row = crate::repo::session::get_row(&conn, session_id).ok()??;
        row.last_cols.zip(row.last_rows)
    }

    pub(super) fn runtime_pid(&self, rt_session: &RuntimeSession) -> Option<i32> {
        self.runtime
            .status(rt_session)
            .ok()
            .flatten()
            .and_then(|status| status.pid)
    }
}

pub(super) fn resolve_spawn_cwd(
    explicit: Option<&str>,
    role_default: Option<&str>,
) -> Option<String> {
    [explicit, role_default]
        .into_iter()
        .flatten()
        .find(|cwd| !cwd.trim().is_empty())
        .map(str::to_owned)
        .or_else(|| {
            runner_core::app_paths::home_dir()
                .and_then(|home| home.into_os_string().into_string().ok())
        })
}

/// Resolve the role config a spawn should actually use (feature 41).
/// Layering is role template, then runtime override, then model/effort
/// overrides. `None` means the role row applies unchanged; a runtime
/// override naming the role's own runtime is no override.
pub(crate) fn resolve_runtime_override(
    role: &Role,
    runtime_override: Option<&str>,
    model_override: Option<&str>,
    effort_override: Option<&str>,
) -> Result<Option<Role>> {
    let runtime_override = runtime_override
        .map(str::trim)
        .filter(|name| !name.is_empty() && *name != role.runtime.as_str());
    let model_override = model_override
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let effort_override = effort_override
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if runtime_override.is_none() && model_override.is_none() && effort_override.is_none() {
        return Ok(None);
    }
    let mut effective = role.clone();
    if let Some(name) = runtime_override {
        let def = crate::runtimes::for_key(name)
            .catalog()
            .ok_or_else(|| Error::msg(format!("unknown runtime: {name}")))?;
        effective.runtime = def.name.to_string();
        effective.command = def.command.to_string();
        effective.args = crate::runtimes::adapter(def.name)
            .permissions()
            .apply(&[], crate::ops::role::default_permission_mode());
        // A differing engine starts from its own defaults; the
        // role's model/effort belong to the original runtime.
        effective.model = None;
        effective.effort = None;
        effective.codex_speed = None;
    }
    if model_override.is_some() {
        effective.model = model_override.map(ToOwned::to_owned);
    }
    if effort_override.is_some() {
        effective.effort = effort_override.map(ToOwned::to_owned);
    }
    Ok(Some(effective))
}

pub(crate) fn runtime_direct_role(
    runtime: &str,
    command: Option<&str>,
    model: Option<&str>,
    effort: Option<&str>,
) -> Result<Role> {
    let runtime = runtime.trim();
    if runtime.is_empty() {
        return Err(Error::msg("runtime is required"));
    }
    let registry = crate::runtimes::for_key(runtime).catalog();
    let command = command
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or_else(|| registry.as_ref().map(|r| r.command))
        .ok_or_else(|| Error::msg(format!("unknown runtime: {runtime}")))?;
    let now = Utc::now();
    let args = if Runtime::parse(runtime).is_some_and(Runtime::is_shell) {
        crate::shell_path::shell_login_args(command)
    } else {
        crate::runtimes::for_key(runtime)
            .permissions()
            .apply(&[], crate::ops::role::default_permission_mode())
    };
    Ok(Role {
        id: format!("runtime:{runtime}"),
        handle: runtime.to_string(),
        display_name: registry
            .map(|r| r.display_name.to_string())
            .unwrap_or_else(|| runtime.to_string()),
        runtime: runtime.to_string(),
        command: command.to_string(),
        args,
        working_dir: None,
        system_prompt: None,
        env: HashMap::new(),
        model: model
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned),
        effort: effort
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned),
        codex_speed: None,
        created_at: now,
        updated_at: now,
    })
}

fn compose_args(
    adapter: &dyn crate::runtimes::RuntimeAdapter,
    role_args: &[String],
    plan: &router::runtime::ResumePlan,
    mission_dir: Option<&Path>,
    ctx: &crate::runtimes::LaunchContext<'_>,
) -> (Vec<String>, bool) {
    let mut args = Vec::new();
    if plan.prepend {
        args.extend(plan.args.iter().cloned());
    }
    args.extend(adapter.launch_role_args(role_args));
    if !plan.prepend {
        args.extend(plan.args.iter().cloned());
    }
    args.extend(adapter.mission_dir_args(mission_dir));
    args.extend(adapter.launch_args(ctx));
    (args, !adapter.first_turn_argv(ctx.first_turn).is_empty())
}

impl SessionManager {
    pub(crate) fn command_preview(
        &self,
        runtime: Runtime,
        command: &str,
        mission: bool,
        options: runner_core::protocol::runtime::RuntimeLaunchOptions,
        app_data_dir: &Path,
        mut env: BTreeMap<String, String>,
    ) -> runner_core::protocol::runtime::RuntimeCommandPreview {
        let adapter = crate::runtimes::adapter(runtime);
        let role = Role {
            id: String::new(),
            handle: String::new(),
            display_name: String::new(),
            runtime: runtime.key().into(),
            command: command.into(),
            args: if mission {
                adapter.permissions().mission_args()
            } else {
                Vec::new()
            },
            working_dir: None,
            system_prompt: None,
            env: HashMap::new(),
            model: Some("<model>".into()),
            effort: Some("<effort>".into()),
            codex_speed: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let role = crate::runtimes::apply_launch_options(&role, mission, options);
        if !mission && !options.status_hooks {
            env.insert(crate::runtimes::STATUS_HOOKS_ENV.into(), "false".into());
        }
        if !options.remote_plugin_catalog {
            env.insert(crate::runtimes::REMOTE_PLUGIN_ENV.into(), "false".into());
        }
        let mut plan = adapter.resume_plan(None);
        if let Some(key) = &plan.assigned_key {
            for arg in &mut plan.args {
                if arg == key {
                    *arg = "<session id>".into();
                }
            }
        }
        let first_turn = if mission {
            "<role prompt, crew notes and mission goal>"
        } else {
            "<first message>"
        };
        let ctx = crate::runtimes::LaunchContext {
            default_model: None,
            env: &env,
            cwd: None,
            role_args: &role.args,
            app_data_dir,
            session_id: "<session id>",
            resuming: false,
            mission,
            model: role.model.as_deref(),
            effort: role.effort.as_deref(),
            codex_speed: None,
            system_prompt: None,
            first_turn: Some(first_turn),
        };
        let (args, _) = compose_args(
            adapter,
            &role.args,
            &plan,
            mission.then_some(Path::new("<mission folder>")),
            &ctx,
        );
        command_lines(runtime, command, &args, mission, options, app_data_dir)
    }
}

fn command_lines(
    runtime: Runtime,
    command: &str,
    args: &[String],
    mission: bool,
    options: runner_core::protocol::runtime::RuntimeLaunchOptions,
    app_data_dir: &Path,
) -> runner_core::protocol::runtime::RuntimeCommandPreview {
    use runner_core::protocol::runtime::{
        RuntimeCommandLine, RuntimeCommandNote as Note, RuntimeCommandPreview,
        RuntimeLaunchOption as OptionKey,
    };
    let mut lines = vec![RuntimeCommandLine {
        command: command.into(),
        note: Note::None,
        option: None,
    }];
    let mut i = 0;
    let mut hooks = Vec::new();
    while i < args.len() {
        let arg = &args[i];
        let takes_value = [
            "-c",
            "--config",
            "--ask-for-approval",
            "--sandbox",
            "--permission-mode",
            "--model",
            "-m",
            "--effort",
            "--thinking",
            "--reasoning-effort",
            "--add-dir",
            "--settings",
            "--enable",
            "-e",
            "--plugin-dir",
            "--log-file",
            "--session-id",
            "--prompt",
            "--",
        ]
        .contains(&arg.as_str());
        let count = if takes_value && i + 1 < args.len() {
            2
        } else {
            1
        };
        let tokens = &args[i..i + count];
        let mut code = tokens.join(" ");
        let mut note = Note::Always;
        let mut option = None;
        if code.contains("<model>") || code.contains("<effort>") {
            note = if mission {
                Note::FromRole
            } else {
                Note::FromChat
            };
        }
        if code.contains("<mission folder>") {
            note = Note::MissionFiles;
        }
        if code.contains("<first message>") || code.contains("<role prompt,") {
            note = if mission {
                Note::FirstTurn
            } else {
                Note::FromChat
            };
        }
        if code.contains("tui.terminal_title") {
            note = Note::NamesChat;
        }
        if code.contains("features.remote_plugin=false") {
            note = Note::CatalogOff;
            option = Some(OptionKey::RemotePluginCatalog);
        }
        if ["--ask-for-approval", "--sandbox", "--permission-mode"].contains(&arg.as_str()) {
            note = if mission {
                Note::AlwaysInMissions
            } else if runtime == Runtime::Codex {
                Note::CodexBypass
            } else {
                Note::ClaudeBypass
            };
            option = (!mission).then_some(OptionKey::Bypass);
        }
        if arg == "--settings" && runtime == Runtime::Antigravity {
            if let [_, settings] = crate::runtimes::antigravity::antigravity_settings_args(
                &[],
                Path::new("<Runner data>"),
            )
            .as_slice()
            {
                code = format!("--settings {settings}");
            }
            note = Note::NamesChat;
        } else if arg == "--settings" {
            if let Some(value) = tokens
                .get(1)
                .and_then(|value| serde_json::from_str::<serde_json::Value>(value).ok())
            {
                let events = value["hooks"].as_object().map_or(0, |hooks| hooks.len());
                code = format!("--settings {{\"tui\":\"fullscreen\",\"theme\":\"auto\",\"hooks\":{{…{events} events}}{}}}", if value["skipDangerousModePermissionPrompt"] == true { ",\"skipDangerousModePermissionPrompt\":true" } else { "" });
            }
            note = if mission {
                Note::AlwaysInMissions
            } else if options.status_hooks && options.bypass {
                Note::HooksBypass
            } else if options.bypass {
                Note::IdentityBypass
            } else if options.status_hooks {
                Note::StatusHooks
            } else {
                Note::Identity
            };
            option = if mission {
                None
            } else if options.status_hooks {
                Some(OptionKey::StatusHooks)
            } else {
                options.bypass.then_some(OptionKey::Bypass)
            };
        }
        if code.contains("hooks.")
            || code.contains("mcp_servers.runner_hooks")
            || code == "--enable hooks"
            || code == "--dangerously-bypass-hook-trust"
        {
            hooks.push(code);
            i += count;
            continue;
        }
        if arg == "-e" || arg == "--plugin-dir" {
            note = if mission {
                Note::AlwaysInMissions
            } else if options.status_hooks {
                Note::StatusHooks
            } else {
                Note::Identity
            };
            option = (!mission && options.status_hooks).then_some(OptionKey::StatusHooks);
        }
        code = code.replace(app_data_dir.to_string_lossy().as_ref(), "<Runner data>");
        let combine = lines.last().is_some_and(|line| {
            (arg == "--sandbox" && line.command.contains("--ask-for-approval"))
                || (code.contains("<effort>") && line.command.contains("<model>"))
        });
        if combine {
            let line = lines.last_mut().unwrap();
            line.command.push(' ');
            line.command.push_str(&code);
        } else {
            lines.push(RuntimeCommandLine {
                command: format!("  {code}"),
                note,
                option,
            });
        }
        i += count;
    }
    if !hooks.is_empty() {
        let count = hooks
            .iter()
            .filter(|line| line.starts_with("-c hooks."))
            .count();
        let note = if mission {
            Note::AlwaysInMissions
        } else if options.status_hooks {
            Note::StatusHooks
        } else {
            Note::Identity
        };
        let option = (!mission && options.status_hooks).then_some(OptionKey::StatusHooks);
        let hook_lines = [
            RuntimeCommandLine {
                command: "  --enable hooks --dangerously-bypass-hook-trust".into(),
                note: note.clone(),
                option,
            },
            RuntimeCommandLine {
                command: format!(
                    "  -c mcp_servers.runner_hooks={{…}} -c hooks.<event>=[…] × {count}"
                ),
                note: note.clone(),
                option,
            },
        ];
        let index = lines
            .iter()
            .position(|line| {
                line.command.contains("<first message>") || line.command.contains("<role prompt,")
            })
            .unwrap_or(lines.len());
        lines.splice(index..index, hook_lines);
    }
    RuntimeCommandPreview { lines }
}

#[cfg(test)]
mod launch_preview_tests {
    use super::*;
    use runner_core::protocol::runtime::RuntimeLaunchOptions;

    #[test]
    fn preview_uses_composed_arguments_and_tracks_switches() {
        let core = crate::test_support::test_core();
        let root = tempfile::tempdir().unwrap();
        let options = RuntimeLaunchOptions {
            bypass: true,
            remote_plugin_catalog: false,
            status_hooks: false,
        };
        let preview = core.sessions.command_preview(
            Runtime::Codex,
            "codex",
            false,
            options,
            root.path(),
            BTreeMap::new(),
        );
        let text = preview
            .lines
            .iter()
            .map(|line| line.command.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        for value in [
            "--ask-for-approval never",
            "--sandbox danger-full-access",
            "--model <model>",
            "model_reasoning_effort=<effort>",
            "features.remote_plugin=false",
            "hooks.<event>=[…] × 2",
            "<first message>",
        ] {
            assert!(text.contains(value), "{value}: {text}");
        }
        let mission = core.sessions.command_preview(
            Runtime::Codex,
            "codex",
            true,
            options,
            root.path(),
            BTreeMap::new(),
        );
        let defaults = core.sessions.command_preview(
            Runtime::Codex,
            "codex",
            true,
            RuntimeLaunchOptions {
                remote_plugin_catalog: false,
                ..Default::default()
            },
            root.path(),
            BTreeMap::new(),
        );
        assert_eq!(
            serde_json::to_value(mission).unwrap(),
            serde_json::to_value(defaults).unwrap()
        );
        let preview = core.sessions.command_preview(
            Runtime::Trae,
            "traecli",
            false,
            RuntimeLaunchOptions::default(),
            root.path(),
            BTreeMap::new(),
        );
        let env = BTreeMap::new();
        let ctx = crate::runtimes::LaunchContext {
            default_model: None,
            env: &env,
            cwd: None,
            role_args: &[],
            app_data_dir: root.path(),
            session_id: "<session id>",
            resuming: false,
            mission: false,
            model: Some("<model>"),
            effort: Some("<effort>"),
            codex_speed: None,
            system_prompt: None,
            first_turn: Some("<first message>"),
        };
        let adapter = crate::runtimes::adapter(Runtime::Trae);
        let (args, _) = compose_args(adapter, &[], &adapter.resume_plan(None), None, &ctx);
        assert_eq!(
            preview.lines[1..]
                .iter()
                .map(|line| line.command.trim())
                .collect::<Vec<_>>()
                .join(" "),
            args.join(" ")
        );

        for mission in [false, true] {
            let agy_preview = core.sessions.command_preview(
                Runtime::Antigravity,
                "agy",
                mission,
                RuntimeLaunchOptions::default(),
                root.path(),
                BTreeMap::new(),
            );
            let agy_text = agy_preview
                .lines
                .iter()
                .map(|line| line.command.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            let normalized_agy = agy_text.replace('\\', "/");
            assert!(
                normalized_agy
                    .contains("--log-file <Runner data>/antigravity/logs/<session id>.log"),
                "{agy_text}"
            );
            assert!(
                !agy_text.contains("antigravity-hooks"),
                "legacy hooks workspace must not appear in preview: {agy_text}"
            );
            assert!(agy_preview.lines.iter().all(|line| {
                line.option
                    != Some(runner_core::protocol::runtime::RuntimeLaunchOption::StatusHooks)
                    && line.note != runner_core::protocol::runtime::RuntimeCommandNote::StatusHooks
            }));
            assert!(!agy_text.contains("--add-dir"), "{agy_text}");
            if mission {
                assert!(agy_text.contains("--dangerously-skip-permissions"));
            }
        }
    }

    #[test]
    fn antigravity_preview_shows_title_hook_settings_without_disturbing_claude() {
        use runner_core::protocol::runtime::{RuntimeCommandNote, RuntimeLaunchOption};

        let core = crate::test_support::test_core();
        let root = tempfile::tempdir().unwrap();
        let app_data = root.path().join("Jason's Runner Data");
        std::fs::create_dir_all(&app_data).unwrap();

        for mission in [false, true] {
            let preview = core.sessions.command_preview(
                Runtime::Antigravity,
                "agy",
                mission,
                RuntimeLaunchOptions::default(),
                &app_data,
                BTreeMap::new(),
            );
            let settings_line = preview
                .lines
                .iter()
                .find(|line| line.command.trim_start().starts_with("--settings "))
                .expect("Antigravity preview should show --settings");
            assert_eq!(settings_line.note, RuntimeCommandNote::NamesChat);
            assert_eq!(settings_line.option, None);
            assert!(!settings_line.command.contains("fullscreen"));
            assert!(settings_line.command.contains("<Runner data>"));
            assert!(!settings_line
                .command
                .contains(app_data.to_string_lossy().as_ref()));
            let json_str = settings_line
                .command
                .trim_start()
                .strip_prefix("--settings ")
                .unwrap();
            let parsed: serde_json::Value = serde_json::from_str(json_str).unwrap();
            assert_eq!(parsed["title"]["type"], "command");
            assert_eq!(parsed["title"]["enabled"], true);
            assert!(parsed["title"]["command"]
                .as_str()
                .unwrap()
                .contains("hook title"));
        }

        let claude = core.sessions.command_preview(
            Runtime::ClaudeCode,
            "claude",
            false,
            RuntimeLaunchOptions {
                bypass: false,
                remote_plugin_catalog: true,
                status_hooks: true,
            },
            &app_data,
            BTreeMap::new(),
        );
        let claude_settings = claude
            .lines
            .iter()
            .find(|line| line.command.trim_start().starts_with("--settings "))
            .expect("Claude Code preview should show --settings");
        assert!(claude_settings.command.contains("\"tui\":\"fullscreen\""));
        assert_eq!(claude_settings.note, RuntimeCommandNote::StatusHooks);
        assert_eq!(
            claude_settings.option,
            Some(RuntimeLaunchOption::StatusHooks)
        );
    }
}
