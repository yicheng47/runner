use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::{
    db,
    error::{Error, Result},
    model::{Role, Runtime},
    AppCore,
};
use runner_core::protocol::{
    ClaudeAccount, ClaudeAccountAction, ClaudeAccounts, ClaudeAuthStatus, ClaudeSessionAccount,
};
use serde::{Deserialize, Serialize};

pub(crate) mod sharing;

const KEY: &str = "claude_accounts";
const NATIVE_ID: &str = "native";

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct AccountsState {
    pub accounts: Vec<ClaudeAccount>,
    pub main_id: String,
    pub active_id: String,
    #[serde(default)]
    pub sharing: BTreeMap<String, sharing::SharedConfig>,
    #[serde(default)]
    pub main_change: Option<sharing::MainChange>,
}

pub struct AccountService {
    authenticate: bool,
    mutations: Mutex<()>,
    statuses: Mutex<BTreeMap<String, ClaudeAuthStatus>>,
    jobs: Mutex<LoginJobs>,
    refreshing: Mutex<bool>,
}

#[derive(Default)]
struct LoginJobs {
    closed: bool,
    entries: HashMap<String, Arc<LoginJob>>,
}

#[derive(Default)]
struct LoginJob {
    process: Mutex<LoginProcess>,
}

#[derive(Default)]
struct LoginProcess {
    cancelled: bool,
    child: Option<(Child, crate::session::process::ProcessTree)>,
}

impl LoginJob {
    fn spawn(&self, mut command: Command) -> Result<Vec<Box<dyn Read + Send>>> {
        let mut process = self.process.lock().unwrap();
        if process.cancelled {
            return Err(Error::msg("sign-in cancelled"));
        }
        let mut child = command.spawn()?;
        let tree = match crate::session::process::ProcessTree::adopt(child.id()) {
            Ok(tree) => tree,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error.into());
            }
        };
        let mut outputs: Vec<Box<dyn Read + Send>> = Vec::new();
        if let Some(stdout) = child.stdout.take() {
            outputs.push(Box::new(stdout));
        }
        if let Some(stderr) = child.stderr.take() {
            outputs.push(Box::new(stderr));
        }
        process.child = Some((child, tree));
        Ok(outputs)
    }

    fn finished(&self) -> Result<bool> {
        let mut process = self.process.lock().unwrap();
        if process.cancelled {
            return Err(Error::msg("sign-in cancelled"));
        }
        Ok(process.child.as_mut().unwrap().0.try_wait()?.is_some())
    }

    fn stop(&self) {
        let mut process = self.process.lock().unwrap();
        process.cancelled = true;
        if let Some((child, tree)) = &mut process.child {
            let _ = tree.terminate();
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Default for AccountService {
    fn default() -> Self {
        Self {
            authenticate: true,
            mutations: Mutex::new(()),
            statuses: Default::default(),
            jobs: Default::default(),
            refreshing: Default::default(),
        }
    }
}

impl AccountService {
    pub fn without_authentication() -> Self {
        Self {
            authenticate: false,
            ..Self::default()
        }
    }
    pub fn stop_logins(&self) {
        let jobs = {
            let mut state = self.jobs.lock().unwrap();
            state.closed = true;
            let jobs = std::mem::take(&mut state.entries);
            let mut statuses = self.statuses.lock().unwrap();
            for id in jobs.keys() {
                statuses.remove(id);
            }
            jobs
        };
        for child in jobs.into_values() {
            child.stop();
        }
    }
}

fn initial(home: &Path) -> AccountsState {
    AccountsState {
        accounts: vec![ClaudeAccount {
            id: NATIVE_ID.into(),
            name: "Personal".into(),
            folder: home.join(".claude").to_string_lossy().into_owned(),
            native: true,
        }],
        main_id: NATIVE_ID.into(),
        active_id: NATIVE_ID.into(),
        sharing: Default::default(),
        main_change: None,
    }
}

pub(crate) fn load(pool: &db::DbPool) -> Result<AccountsState> {
    let conn = pool.get()?;
    load_at(
        &conn,
        &runner_core::app_paths::home_dir()
            .ok_or_else(|| Error::msg("home directory unavailable"))?,
    )
}

fn load_at(conn: &rusqlite::Connection, home: &Path) -> Result<AccountsState> {
    Ok(db::app_state_get(conn, KEY)?
        .map(|json| serde_json::from_str(&json))
        .transpose()?
        .unwrap_or_else(|| initial(home)))
}

pub fn snapshot(core: &AppCore) -> Result<ClaudeAccounts> {
    let state = load(&core.db)?;
    Ok(ClaudeAccounts {
        sharing: sharing::summaries(&state),
        pending_main_id: state
            .main_change
            .as_ref()
            .map(|change| change.target_id.clone()),
        accounts: state.accounts,
        main_id: state.main_id,
        active_id: state.active_id,
        statuses: core.claude_accounts.statuses.lock().unwrap().clone(),
    })
}

fn name(value: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(Error::msg("account name is required"));
    }
    Ok(value.to_owned())
}

fn mutate(
    state: &mut AccountsState,
    action: &ClaudeAccountAction,
    statuses: &BTreeMap<String, ClaudeAuthStatus>,
    home: &Path,
) -> Result<Option<String>> {
    let mut created_id = None;
    match action {
        ClaudeAccountAction::Add {
            name: value,
            folder,
            ..
        }
        | ClaudeAccountAction::AddAndLogin {
            name: value,
            folder,
            ..
        } => {
            let name = name(value)?;
            let folder = folder
                .as_ref()
                .map(|folder| {
                    folder
                        .strip_prefix("~/")
                        .map(|rest| home.join(rest))
                        .unwrap_or_else(|| PathBuf::from(folder))
                })
                .unwrap_or_else(|| {
                    home.join(runner_core::protocol::claude_account_folder_name(&name))
                });
            if !folder.is_absolute() {
                return Err(Error::msg("account folder must be absolute"));
            }
            let folder = folder.to_string_lossy().into_owned();
            if state
                .accounts
                .iter()
                .any(|account| account.folder == folder)
            {
                return Err(Error::msg("this folder already has an account"));
            }
            let id = ulid::Ulid::new().to_string();
            created_id = Some(id.clone());
            state.accounts.push(ClaudeAccount {
                id,
                name,
                folder,
                native: false,
            });
        }
        ClaudeAccountAction::Rename { id, name: value } => {
            let value = name(value)?;
            state
                .accounts
                .iter_mut()
                .find(|account| &account.id == id)
                .ok_or_else(|| Error::msg("account not found"))?
                .name = value;
        }
        ClaudeAccountAction::Remove { id } => {
            if id == &state.main_id
                || id == &state.active_id
                || state
                    .accounts
                    .iter()
                    .any(|account| &account.id == id && account.native)
            {
                return Err(Error::msg(
                    "the main, active and original native accounts cannot be removed",
                ));
            }
            state.accounts.retain(|account| &account.id != id);
        }
        ClaudeAccountAction::MakeMain { id } | ClaudeAccountAction::Use { id } => {
            if !state.accounts.iter().any(|account| &account.id == id) {
                return Err(Error::msg("account not found"));
            }
            if matches!(action, ClaudeAccountAction::Use { .. }) {
                if !statuses.get(id).is_some_and(|status| status.logged_in) {
                    return Err(Error::msg("sign in before using this account"));
                }
                state.active_id = id.clone();
            } else {
                state.main_id = id.clone();
            }
        }
        _ => {}
    }
    Ok(created_id)
}

pub fn action(core: &AppCore, action: ClaudeAccountAction) -> Result<ClaudeAccounts> {
    match &action {
        ClaudeAccountAction::Refresh => {
            refresh(core);
            return snapshot(core);
        }
        ClaudeAccountAction::Login { id } => {
            let _mutation = core.claude_accounts.mutations.lock().unwrap();
            let account = load(&core.db)?
                .accounts
                .into_iter()
                .find(|account| &account.id == id)
                .ok_or_else(|| Error::msg("account not found"))?;
            login(core, account)?;
            return snapshot(core);
        }
        ClaudeAccountAction::Cancel { id } => {
            let _mutation = core.claude_accounts.mutations.lock().unwrap();
            if let Some(child) = core.claude_accounts.jobs.lock().unwrap().entries.remove(id) {
                child.stop();
            }
            core.claude_accounts.statuses.lock().unwrap().remove(id);
            core.events.emit("claude_accounts/changed", &());
            return snapshot(core);
        }
        _ => {}
    }
    let _mutation = core.claude_accounts.mutations.lock().unwrap();
    let mut conn = core.db.get()?;
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let home = runner_core::app_paths::home_dir()
        .ok_or_else(|| Error::msg("home directory unavailable"))?;
    let mut state = load_at(&tx, &home)?;
    sharing::validate_action(&state, &action)?;
    let previous = state.clone();
    let previous_active = state.active_id.clone();
    let was_multiple = state.accounts.len() > 1;
    let created_id = mutate(
        &mut state,
        &action,
        &core.claude_accounts.statuses.lock().unwrap(),
        &home,
    )?;
    let sharing_result =
        sharing::apply_action(&mut state, &previous, &action, created_id.as_deref());
    if sharing_result.is_err() && matches!(action, ClaudeAccountAction::Remove { .. }) {
        state.accounts = previous.accounts.clone();
    }
    db::app_state_set(&tx, KEY, &serde_json::to_string(&state)?)?;
    tx.commit()?;
    drop(conn);
    let added = created_id.is_some();
    if sharing_result.is_ok() && matches!(action, ClaudeAccountAction::AddAndLogin { .. }) {
        let id = created_id.as_ref().unwrap();
        let account = state
            .accounts
            .iter()
            .find(|account| &account.id == id)
            .unwrap();
        login(core, account.clone())?;
    }
    if previous_active != state.active_id || was_multiple != (state.accounts.len() > 1) {
        core.usage
            .select_claude_account(&state.active_id, state.accounts.len() > 1);
    }
    if let ClaudeAccountAction::Remove { id } = &action {
        if let Some(child) = core.claude_accounts.jobs.lock().unwrap().entries.remove(id) {
            child.stop();
        }
        core.claude_accounts.statuses.lock().unwrap().remove(id);
    }
    core.events.emit("claude_accounts/changed", &());
    if previous.main_id != state.main_id {
        crate::runtime_status::models::request(
            &core.db,
            &core.runtime_shell_env,
            &core.runtime_discovery,
            &core.events,
            &[Runtime::ClaudeCode],
            true,
        );
        crate::runtime_status::versions::request_latest(
            &core.db,
            &core.runtime_shell_env,
            &core.runtime_discovery,
            &core.events,
            true,
        );
        core.events.emit("runtime/changed", &());
    }
    if added {
        refresh(core);
    } else {
        core.usage.request_account_refresh(core.clone());
    }
    sharing_result?;
    snapshot(core)
}

fn command(core: &AppCore, account: &ClaudeAccount) -> Result<Command> {
    let statuses = crate::runtime_status::status_list(
        &core.db,
        &core.runtime_shell_env,
        &core.runtime_discovery,
    )?;
    let executable = statuses
        .runtimes
        .into_iter()
        .find(|status| status.name == Runtime::ClaudeCode)
        .and_then(|status| status.effective_command)
        .ok_or_else(|| Error::msg("Claude Code is not installed"))?;
    let mut command = Command::new(executable);
    let env = core.runtime_shell_env.read().unwrap();
    command
        .envs(&env.vars)
        .env("PATH", crate::runtime_status::direct_chat_path(&env));
    crate::session::process::prepare_headless_fork(&mut command);
    if account.native {
        command.env_remove("CLAUDE_CONFIG_DIR");
    } else {
        command.env("CLAUDE_CONFIG_DIR", &account.folder);
    }
    command.env_remove("CLAUDECODE");
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    Ok(command)
}

fn parse_status(bytes: &[u8]) -> ClaudeAuthStatus {
    let value: serde_json::Value = match serde_json::from_slice(bytes) {
        Ok(value) => value,
        Err(_) => {
            return ClaudeAuthStatus {
                error: true,
                ..Default::default()
            }
        }
    };
    let field = |key| {
        value
            .get(key)
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    };
    ClaudeAuthStatus {
        logged_in: value
            .get("loggedIn")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        email: field("email"),
        organization: field("orgName"),
        plan: field("subscriptionType"),
        ..Default::default()
    }
}

fn auth_status(core: &AppCore, account: &ClaudeAccount) -> ClaudeAuthStatus {
    auth_status_for_job(core, account, &LoginJob::default())
}

fn auth_status_for_job(
    core: &AppCore,
    account: &ClaudeAccount,
    job: &LoginJob,
) -> ClaudeAuthStatus {
    let result = (|| -> Result<Vec<u8>> {
        let mut command = command(core, account)?;
        command.args(["auth", "status"]);
        let mut outputs = job.spawn(command)?;
        let mut stdout = outputs.remove(0);
        let reader = std::thread::spawn(move || {
            let mut bytes = Vec::new();
            stdout.read_to_end(&mut bytes).map(|_| bytes)
        });
        let deadline = Instant::now() + Duration::from_secs(15);
        while !job.finished()? {
            if Instant::now() >= deadline {
                job.stop();
                return Err(Error::msg("auth status timed out"));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        Ok(reader
            .join()
            .map_err(|_| Error::msg("auth status unavailable"))??)
    })();
    result
        .map(|bytes| parse_status(&bytes))
        .unwrap_or_else(|_| ClaudeAuthStatus {
            error: true,
            ..Default::default()
        })
}

pub(crate) fn refresh(core: &AppCore) {
    if !core.claude_accounts.authenticate {
        return;
    }
    let mut refreshing = core.claude_accounts.refreshing.lock().unwrap();
    if *refreshing {
        return;
    }
    *refreshing = true;
    let core = core.clone();
    std::thread::spawn(move || {
        refresh_statuses_inner(&core);
        *core.claude_accounts.refreshing.lock().unwrap() = false;
        if load(&core.db).is_ok_and(|state| state.accounts.len() > 1) {
            core.usage.request_account_refresh(core.clone());
        }
    });
}

pub(crate) fn refresh_for_usage(core: &AppCore) {
    if !core.claude_accounts.authenticate {
        return;
    }
    let mut refreshing = core.claude_accounts.refreshing.lock().unwrap();
    if *refreshing {
        return;
    }
    *refreshing = true;
    drop(refreshing);
    refresh_statuses_inner(core);
    *core.claude_accounts.refreshing.lock().unwrap() = false;
}

fn refresh_statuses_inner(core: &AppCore) {
    if let Ok(state) = load(&core.db) {
        for account in state.accounts {
            if core
                .claude_accounts
                .jobs
                .lock()
                .unwrap()
                .entries
                .contains_key(&account.id)
            {
                continue;
            }
            let status = auth_status(core, &account);
            let _mutation = core.claude_accounts.mutations.lock().unwrap();
            if !load(&core.db).is_ok_and(|state| {
                state
                    .accounts
                    .iter()
                    .any(|current| current.id == account.id)
            }) {
                continue;
            }
            let jobs = core.claude_accounts.jobs.lock().unwrap();
            if !jobs.closed && !jobs.entries.contains_key(&account.id) {
                let status = ready_status(&account, status);
                core.claude_accounts
                    .statuses
                    .lock()
                    .unwrap()
                    .insert(account.id, status);
                core.events.emit("claude_accounts/changed", &());
            }
        }
    }
}

fn login(core: &AppCore, account: ClaudeAccount) -> Result<()> {
    if !core.claude_accounts.authenticate {
        return Err(Error::msg("authentication is disabled in this test core"));
    }
    let mut jobs = core.claude_accounts.jobs.lock().unwrap();
    if jobs.closed {
        return Err(Error::msg("daemon is shutting down"));
    }
    if jobs.entries.contains_key(&account.id) {
        return Ok(());
    }
    let job = Arc::new(LoginJob::default());
    jobs.entries.insert(account.id.clone(), job.clone());
    core.claude_accounts.statuses.lock().unwrap().insert(
        account.id.clone(),
        ClaudeAuthStatus {
            signing_in: true,
            ..Default::default()
        },
    );
    drop(jobs);
    core.events.emit("claude_accounts/changed", &());
    let core = core.clone();
    std::thread::spawn(move || {
        let status = auth_status_for_job(&core, &account, &job);
        if status.logged_in {
            finish_login(&core, &account, &job, status);
            return;
        }
        let result = (|| -> Result<()> {
            let outputs = {
                let _mutation = core.claude_accounts.mutations.lock().unwrap();
                if !core
                    .claude_accounts
                    .jobs
                    .lock()
                    .unwrap()
                    .entries
                    .get(&account.id)
                    .is_some_and(|current| Arc::ptr_eq(current, &job))
                {
                    return Ok(());
                }
                let mut command = command(&core, &account)?;
                command.args(["auth", "login"]).stderr(Stdio::piped());
                job.spawn(command)?
            };
            login_outputs(&core, &account.id, &job, outputs);
            while !job.finished()? {
                std::thread::sleep(Duration::from_millis(100));
            }
            let status = auth_status_for_job(&core, &account, &job);
            finish_login(&core, &account, &job, status);
            Ok(())
        })();
        if result.is_err() {
            finish_login(
                &core,
                &account,
                &job,
                ClaudeAuthStatus {
                    error: true,
                    ..Default::default()
                },
            );
        }
    });
    Ok(())
}

fn ready_status(account: &ClaudeAccount, status: ClaudeAuthStatus) -> ClaudeAuthStatus {
    if status.logged_in && !account.native {
        let result = runner_core::app_paths::home_dir()
            .ok_or_else(|| Error::msg("home directory unavailable"))
            .and_then(|home| crate::runtimes::claude_code::onboarding::complete(account, &home));
        if result.is_err() {
            log::warn!("Claude Code account onboarding could not be prepared");
            return ClaudeAuthStatus {
                error: true,
                ..Default::default()
            };
        }
    }
    status
}

fn finish_login(
    core: &AppCore,
    account: &ClaudeAccount,
    job: &Arc<LoginJob>,
    status: ClaudeAuthStatus,
) {
    let id = account.id.as_str();
    let _mutation = core.claude_accounts.mutations.lock().unwrap();
    let mut jobs = core.claude_accounts.jobs.lock().unwrap();
    if !jobs
        .entries
        .get(id)
        .is_some_and(|current| Arc::ptr_eq(current, job))
    {
        return;
    }
    let status = ready_status(account, status);
    jobs.entries.remove(id);
    let signed_in = status.logged_in;
    core.claude_accounts
        .statuses
        .lock()
        .unwrap()
        .insert(id.to_owned(), status);
    drop(jobs);
    core.events.emit("claude_accounts/changed", &());
    if signed_in {
        core.usage.request_account_refresh(core.clone());
    }
}

fn login_outputs(
    core: &AppCore,
    id: &str,
    child: &Arc<LoginJob>,
    outputs: Vec<Box<dyn Read + Send>>,
) {
    for output in outputs {
        let stream_core = core.clone();
        let stream_child = child.clone();
        let stream_id = id.to_owned();
        std::thread::spawn(move || {
            for line in BufReader::new(output)
                .lines()
                .map_while(std::result::Result::ok)
            {
                let Some(start) = line.find("https://") else {
                    continue;
                };
                let url = line[start..]
                    .split(|character: char| character.is_whitespace() || character.is_control())
                    .next()
                    .unwrap();
                let Ok(parsed) = reqwest::Url::parse(url) else {
                    continue;
                };
                if !matches!(
                    parsed.host_str(),
                    Some("claude.ai" | "platform.claude.com" | "console.anthropic.com")
                ) || !parsed.path().starts_with("/oauth/")
                {
                    continue;
                }
                let jobs = stream_core.claude_accounts.jobs.lock().unwrap();
                if !jobs
                    .entries
                    .get(&stream_id)
                    .is_some_and(|child| Arc::ptr_eq(child, &stream_child))
                {
                    continue;
                }
                if let Some(status) = stream_core
                    .claude_accounts
                    .statuses
                    .lock()
                    .unwrap()
                    .get_mut(&stream_id)
                {
                    status.sign_in_url = Some(url.to_owned());
                }
                stream_core.events.emit("claude_accounts/changed", &());
            }
        });
    }
}

pub(crate) fn config_pin(role: &Role, shell: &BTreeMap<String, String>) -> Option<String> {
    use crate::runtimes::claude_code::{agent_config_dir, spawned_config_dir};
    agent_config_dir(spawned_config_dir(shell.iter().chain(&role.env)))?
        .into_string()
        .ok()
}

pub(crate) fn launch_account(
    pool: &db::DbPool,
    role: &mut Role,
    shell: &BTreeMap<String, String>,
    previous: Option<&ClaudeSessionAccount>,
    fresh: bool,
) -> Result<Option<ClaudeSessionAccount>> {
    if Runtime::parse(&role.runtime) != Some(Runtime::ClaudeCode) {
        return Ok(None);
    }
    let state = load(pool)?;
    if state.accounts.len() <= 1 && previous.is_none() {
        return Ok(None);
    }
    let pin = config_pin(role, shell);
    let legacy_origin = pin.clone();
    let pinned = pin.is_some();
    let (folder, native, account_id) = if let Some(pin) = pin {
        let matched = state
            .accounts
            .iter()
            .find(|account| !account.native && account.folder == pin);
        (pin, false, matched.map(|account| account.id.clone()))
    } else {
        let account = state
            .accounts
            .iter()
            .find(|account| account.id == state.active_id)
            .ok_or_else(|| Error::msg("active account missing"))?;
        if !account.native {
            role.env
                .insert("CLAUDE_CONFIG_DIR".into(), account.folder.clone());
        }
        (
            account.folder.clone(),
            account.native,
            Some(account.id.clone()),
        )
    };
    let origin_folder = if fresh {
        folder.clone()
    } else {
        previous
            .map(|previous| previous.origin_folder.clone())
            .or(legacy_origin)
            .unwrap_or_else(|| {
                state
                    .accounts
                    .iter()
                    .find(|account| account.native)
                    .unwrap()
                    .folder
                    .clone()
            })
    };
    Ok(Some(ClaudeSessionAccount {
        account_id,
        folder,
        native,
        pinned,
        origin_folder,
        permission_mode: previous
            .map(|previous| previous.permission_mode.clone())
            .unwrap_or_else(|| "default".into()),
    }))
}

pub(crate) fn transcript(folder: &str, cwd: Option<&str>, key: &str) -> Option<PathBuf> {
    uuid::Uuid::parse_str(key).ok()?;
    let cwd = cwd?;
    let path = |cwd: &str| {
        Path::new(folder)
            .join("projects")
            .join(crate::runtimes::claude_code::claude_code_project_dir(cwd))
            .join(format!("{key}.jsonl"))
    };
    let original = path(cwd);
    if original.is_file() {
        return Some(original);
    }
    if let Ok(canonical) = std::fs::canonicalize(cwd) {
        #[cfg(windows)]
        let canonical = crate::runtimes::ordinary_windows_path(&canonical);
        let canonical = path(&canonical.to_string_lossy());
        if canonical.is_file() {
            return Some(canonical);
        }
    }
    Some(original)
}

pub(crate) fn resume_args(
    args: &mut Vec<String>,
    account: &ClaudeSessionAccount,
    origin: &str,
    cwd: Option<&str>,
    key: &str,
) {
    if origin == account.folder {
        return;
    }
    if let Some(path) = transcript(origin, cwd, key) {
        if let Some(position) = args.iter().position(|arg| arg == "--resume") {
            args[position + 1] = path.to_string_lossy().into_owned();
        }
        args.extend(["--permission-mode".into(), account.permission_mode.clone()]);
    }
}

pub(crate) fn last_permission(
    origin: &str,
    cwd: Option<&str>,
    key: &str,
    fallback: &str,
) -> String {
    let Some(reader) = transcript(origin, cwd, key)
        .and_then(|path| std::fs::File::open(path).ok())
        .map(BufReader::new)
    else {
        return fallback.to_owned();
    };
    let mut mode = fallback.to_owned();
    for line in reader.lines().map_while(std::result::Result::ok) {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) {
            if let Some(value) = value
                .get("permissionMode")
                .and_then(serde_json::Value::as_str)
                .filter(|value| {
                    matches!(
                        *value,
                        "default"
                            | "acceptEdits"
                            | "auto"
                            | "bypassPermissions"
                            | "plan"
                            | "dontAsk"
                    )
                })
            {
                mode = value.to_owned();
            }
        }
    }
    mode
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn slow_login_core(temp: &Path) -> AppCore {
        use std::os::unix::fs::PermissionsExt;
        let script = temp.join("slow-claude");
        std::fs::write(
            &script,
            r#"#!/bin/sh
if [ "$2" = status ]; then
    if [ -n "$CLAUDE_CONFIG_DIR" ]; then
        printf '%s' "$$" > "$CLAUDE_CONFIG_DIR/status-started"
        while [ ! -f "$CLAUDE_CONFIG_DIR/release-status" ]; do sleep 0.01; done
    fi
    printf '%s\n' '{"loggedIn":false}'
    exit 1
fi
printf '%s' "$CLAUDE_CONFIG_DIR" > "$CLAUDE_CONFIG_DIR/login-started"
printf '%s\n' 'https://claude.ai/oauth/authorize?fixture=1'
exec sleep 60
"#,
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut core = crate::test_support::test_core_in(temp.to_owned());
        core.claude_accounts = Default::default();
        core.usage = Arc::new(crate::usage::UsageService::without_fetch());
        db::set_runtime_override(&core.db, "claude-code", script.to_str()).unwrap();
        db::app_state_set(
            &core.db.get().unwrap(),
            KEY,
            &serde_json::to_string(&initial(temp)).unwrap(),
        )
        .unwrap();
        core
    }

    #[cfg(unix)]
    fn wait_for(description: &str, predicate: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !predicate() {
            assert!(Instant::now() < deadline, "timed out: {description}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[cfg(unix)]
    #[test]
    fn add_and_login_keeps_each_request_bound_to_its_created_account() {
        let temp = tempfile::tempdir().unwrap();
        let core = slow_login_core(temp.path());
        let before_a = snapshot(&core).unwrap();
        let before_b = snapshot(&core).unwrap();
        assert_eq!(before_a.accounts, before_b.accounts);
        let mut added = Vec::new();
        // B wins admission while A still has the same stale initial snapshot.
        for name in ["B", "A"] {
            let folder = temp.path().join(name);
            std::fs::create_dir(&folder).unwrap();
            let response = action(
                &core,
                ClaudeAccountAction::AddAndLogin {
                    name: name.into(),
                    folder: Some(folder.to_string_lossy().into_owned()),
                    use_main_settings: false,
                },
            )
            .unwrap();
            let id = response
                .accounts
                .iter()
                .find(|account| account.name == name)
                .unwrap()
                .id
                .clone();
            assert!(response.statuses[&id].signing_in);
            wait_for("pending status probe", || {
                folder.join("status-started").is_file()
            });
            added.push((id, folder));
        }
        assert_eq!(snapshot(&core).unwrap().accounts.len(), 3);
        for (id, folder) in &added {
            std::fs::write(folder.join("release-status"), "").unwrap();
            // The shell redirect creates the file before printf fills it.
            wait_for("matching account login", || {
                std::fs::read_to_string(folder.join("login-started"))
                    .is_ok_and(|started| !started.is_empty())
            });
            assert_eq!(
                std::fs::read_to_string(folder.join("login-started")).unwrap(),
                folder.to_string_lossy()
            );
            assert!(snapshot(&core).unwrap().statuses[id].signing_in);
        }
        core.claude_accounts.stop_logins();
    }

    #[cfg(unix)]
    #[test]
    fn signed_in_add_and_completed_login_preseed_only_the_onboarding_flag() {
        use std::os::unix::fs::PermissionsExt;
        for already_signed_in in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let core = slow_login_core(temp.path());
            let script = temp.path().join("signed-in-claude");
            std::fs::write(
                &script,
                r#"#!/bin/sh
if [ "$2" = status ]; then
    if [ -f "$CLAUDE_CONFIG_DIR/signed-in" ]; then
        printf '%s\n' '{"loggedIn":true}'
    else
        printf '%s\n' '{"loggedIn":false}'
    fi
else
    touch "$CLAUDE_CONFIG_DIR/signed-in" "$CLAUDE_CONFIG_DIR/login-started"
fi
"#,
            )
            .unwrap();
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
            db::set_runtime_override(&core.db, "claude-code", script.to_str()).unwrap();
            let folder = temp.path().join("work");
            std::fs::create_dir(&folder).unwrap();
            if already_signed_in {
                std::fs::write(folder.join("signed-in"), "").unwrap();
            }
            let original = "{\"untouched\":1e+02,\"escaped\":\"\\u0041\"}";
            std::fs::write(folder.join(".claude.json"), original).unwrap();
            let response = action(
                &core,
                ClaudeAccountAction::AddAndLogin {
                    name: "Work".into(),
                    folder: Some(folder.to_string_lossy().into_owned()),
                    use_main_settings: false,
                },
            )
            .unwrap();
            let id = response.accounts.last().unwrap().id.clone();
            wait_for("signed-in account becomes ready", || {
                snapshot(&core)
                    .unwrap()
                    .statuses
                    .get(&id)
                    .is_some_and(|status| status.logged_in)
            });
            assert_eq!(folder.join("login-started").exists(), !already_signed_in);
            let raw = std::fs::read_to_string(folder.join(".claude.json")).unwrap();
            assert_eq!(
                raw,
                "{\"untouched\":1e+02,\"escaped\":\"\\u0041\",\"hasCompletedOnboarding\":true}"
            );
            assert!(!raw.contains("lastOnboardingVersion"));
            core.claude_accounts.stop_logins();
        }
    }

    #[cfg(unix)]
    #[test]
    fn shutdown_rejects_login_requests_delayed_before_registration() {
        for add in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let core = slow_login_core(temp.path());
            *core.claude_accounts.refreshing.lock().unwrap() = true;
            let folder = temp.path().join("work");
            std::fs::create_dir(&folder).unwrap();
            let request = if add {
                ClaudeAccountAction::AddAndLogin {
                    name: "Work".into(),
                    folder: Some(folder.to_string_lossy().into_owned()),
                    use_main_settings: false,
                }
            } else {
                let added = action(
                    &core,
                    ClaudeAccountAction::Add {
                        name: "Work".into(),
                        folder: Some(folder.to_string_lossy().into_owned()),
                        use_main_settings: false,
                    },
                )
                .unwrap();
                ClaudeAccountAction::Login {
                    id: added.accounts.last().unwrap().id.clone(),
                }
            };
            let admission = core.claude_accounts.mutations.lock().unwrap();
            let (dispatched, ready) = std::sync::mpsc::channel();
            let request_core = core.clone();
            let pending = std::thread::spawn(move || {
                dispatched.send(()).unwrap();
                action(&request_core, request)
            });
            ready.recv_timeout(Duration::from_secs(5)).unwrap();
            core.claude_accounts.stop_logins();
            drop(admission);
            let response = pending.join().unwrap();
            core.claude_accounts.stop_logins();
            assert_eq!(
                response
                    .expect_err("shutdown accepted delayed request")
                    .to_string(),
                "daemon is shutting down",
                "add={add}"
            );
            assert!(!folder.join("status-started").exists());
            assert!(!folder.join("login-started").exists());
            assert!(core.claude_accounts.jobs.lock().unwrap().entries.is_empty());
            assert!(snapshot(&core).unwrap().statuses.is_empty());
        }
    }

    #[cfg(unix)]
    #[test]
    fn remove_and_cancel_stop_pending_login_before_it_can_start_authentication() {
        for remove in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let core = slow_login_core(temp.path());
            // Suppress unrelated automatic refresh so the only status subprocess is this request's preflight.
            *core.claude_accounts.refreshing.lock().unwrap() = true;
            let folder = temp.path().join("work");
            std::fs::create_dir(&folder).unwrap();
            let added = action(
                &core,
                ClaudeAccountAction::Add {
                    name: "Work".into(),
                    folder: Some(folder.to_string_lossy().into_owned()),
                    use_main_settings: false,
                },
            )
            .unwrap();
            let id = added.accounts.last().unwrap().id.clone();
            action(&core, ClaudeAccountAction::Login { id: id.clone() }).unwrap();
            let job = core.claude_accounts.jobs.lock().unwrap().entries[&id].clone();
            wait_for("slow preflight starts", || {
                folder.join("status-started").is_file()
            });
            assert!(snapshot(&core).unwrap().statuses[&id].signing_in);
            let response = action(
                &core,
                if remove {
                    ClaudeAccountAction::Remove { id: id.clone() }
                } else {
                    ClaudeAccountAction::Cancel { id: id.clone() }
                },
            )
            .unwrap();
            assert!(!response.statuses.contains_key(&id));
            std::fs::write(folder.join("release-status"), "").unwrap();
            wait_for("cancelled worker exits", || Arc::strong_count(&job) == 1);
            assert!(!folder.join("login-started").exists());
            assert!(!core
                .claude_accounts
                .jobs
                .lock()
                .unwrap()
                .entries
                .contains_key(&id));
            assert!(!snapshot(&core).unwrap().statuses.contains_key(&id));
            assert_eq!(
                load(&core.db)
                    .unwrap()
                    .accounts
                    .iter()
                    .any(|account| account.id == id),
                !remove
            );
            let mut process = job.process.lock().unwrap();
            assert!(process.cancelled);
            assert!(process
                .child
                .as_mut()
                .unwrap()
                .0
                .try_wait()
                .unwrap()
                .is_some());
        }
    }

    #[cfg(unix)]
    #[test]
    fn login_uses_the_selected_folder_and_cancel_stops_only_its_process() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let script = temp.path().join("fake-claude");
        let recorded = temp.path().join("login-folder");
        std::fs::write(&script,format!("#!/bin/sh\nif [ \"$2\" = status ]; then printf '%s\\n' '{{\"loggedIn\":false}}'; exit 1; fi\nprintf '%s' \"${{CLAUDE_CONFIG_DIR-unset}}\" > '{}'\nprintf '%s\\n' 'https://claude.ai/oauth/authorize?fixture=1'\nexec sleep 60\n",recorded.display())).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut core = crate::test_support::test_core_in(temp.path().to_owned());
        core.claude_accounts = Default::default();
        core.usage = Arc::new(crate::usage::UsageService::without_fetch());
        db::set_runtime_override(&core.db, "claude-code", script.to_str()).unwrap();
        let mut state = initial(temp.path());
        state.accounts.push(ClaudeAccount {
            id: "work".into(),
            name: "Work".into(),
            folder: temp
                .path()
                .join("work folder")
                .to_string_lossy()
                .into_owned(),
            native: false,
        });
        db::app_state_set(
            &core.db.get().unwrap(),
            KEY,
            &serde_json::to_string(&state).unwrap(),
        )
        .unwrap();
        action(&core, ClaudeAccountAction::Login { id: "work".into() }).unwrap();
        let child = core.claude_accounts.jobs.lock().unwrap().entries["work"].clone();
        let deadline = Instant::now() + Duration::from_secs(3);
        while !core
            .claude_accounts
            .statuses
            .lock()
            .unwrap()
            .get("work")
            .is_some_and(|status| status.sign_in_url.is_some())
        {
            assert!(
                Instant::now() < deadline,
                "fake login did not publish its URL"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            std::fs::read_to_string(&recorded).unwrap(),
            state.accounts[1].folder
        );
        assert!(snapshot(&core).unwrap().statuses["work"].signing_in);
        action(&core, ClaudeAccountAction::Cancel { id: "work".into() }).unwrap();
        assert!(!core
            .claude_accounts
            .jobs
            .lock()
            .unwrap()
            .entries
            .contains_key("work"));
        let mut process = child.process.lock().unwrap();
        assert!(process.cancelled);
        assert!(process.child.as_mut().unwrap().0.wait().unwrap().code() != Some(0));
    }

    #[test]
    fn account_operations_preserve_sign_in_targets_and_user_files() {
        let home = tempfile::tempdir().unwrap();
        let mut state = initial(home.path());
        let mut statuses = BTreeMap::new();
        let add = ClaudeAccountAction::Add {
            name: "  Work Team  ".into(),
            folder: None,
            use_main_settings: false,
        };
        let _ = mutate(&mut state, &add, &statuses, home.path()).unwrap();
        let work = state.accounts[1].clone();
        assert!(work.folder.ends_with(".claude-work-team"));
        assert!(!work.native);
        assert!(mutate(
            &mut state,
            &ClaudeAccountAction::Use {
                id: work.id.clone()
            },
            &statuses,
            home.path()
        )
        .is_err());
        statuses.insert(
            work.id.clone(),
            parse_status(br#"{"loggedIn":true,"subscriptionType":null}"#),
        );
        let _ = mutate(
            &mut state,
            &ClaudeAccountAction::Use {
                id: work.id.clone(),
            },
            &statuses,
            home.path(),
        )
        .unwrap();
        assert!(mutate(
            &mut state,
            &ClaudeAccountAction::Remove {
                id: work.id.clone()
            },
            &statuses,
            home.path()
        )
        .is_err());
        let _ = mutate(
            &mut state,
            &ClaudeAccountAction::MakeMain {
                id: work.id.clone(),
            },
            &statuses,
            home.path(),
        )
        .unwrap();
        assert_eq!(state.accounts[1], work);
        assert!(state.accounts[0].native);
        assert!(mutate(
            &mut state,
            &ClaudeAccountAction::Remove {
                id: NATIVE_ID.into()
            },
            &statuses,
            home.path()
        )
        .is_err());
        let _ = mutate(
            &mut state,
            &ClaudeAccountAction::Rename {
                id: work.id.clone(),
                name: "Office".into(),
            },
            &statuses,
            home.path(),
        )
        .unwrap();
        assert_eq!(state.accounts[1].folder, work.folder);
        statuses.insert(
            NATIVE_ID.into(),
            ClaudeAuthStatus {
                logged_in: true,
                ..Default::default()
            },
        );
        let _ = mutate(
            &mut state,
            &ClaudeAccountAction::Use {
                id: NATIVE_ID.into(),
            },
            &statuses,
            home.path(),
        )
        .unwrap();
        let _ = mutate(
            &mut state,
            &ClaudeAccountAction::MakeMain {
                id: NATIVE_ID.into(),
            },
            &statuses,
            home.path(),
        )
        .unwrap();
        std::fs::create_dir_all(&work.folder).unwrap();
        let file = Path::new(&work.folder).join("user-file");
        std::fs::write(&file, "preserve").unwrap();
        let _ = mutate(
            &mut state,
            &ClaudeAccountAction::Remove { id: work.id },
            &statuses,
            home.path(),
        )
        .unwrap();
        assert_eq!(state.accounts.len(), 1);
        assert_eq!(std::fs::read_to_string(file).unwrap(), "preserve");
        assert!(mutate(
            &mut state,
            &ClaudeAccountAction::Add {
                name: "".into(),
                folder: None,
                use_main_settings: false,
            },
            &statuses,
            home.path()
        )
        .is_err());
        assert!(mutate(
            &mut state,
            &ClaudeAccountAction::Add {
                name: "Other".into(),
                folder: Some("relative".into()),
                use_main_settings: false,
            },
            &statuses,
            home.path()
        )
        .is_err());
    }

    #[cfg(unix)]
    #[test]
    fn transcript_lookup_keeps_the_existing_canonical_cwd_fallback() {
        let temp = tempfile::tempdir().unwrap();
        let cwd = temp.path().join("physical");
        std::fs::create_dir(&cwd).unwrap();
        let alias = temp.path().join("alias");
        std::os::unix::fs::symlink(&cwd, &alias).unwrap();
        let key = uuid::Uuid::new_v4().to_string();
        let folder = temp.path().join("account").to_string_lossy().into_owned();
        let physical = cwd.canonicalize().unwrap();
        let actual = transcript(&folder, physical.to_str(), &key).unwrap();
        std::fs::create_dir_all(actual.parent().unwrap()).unwrap();
        std::fs::write(&actual, "{}").unwrap();
        assert_eq!(transcript(&folder, alias.to_str(), &key), Some(actual));
    }

    #[test]
    fn status_null_plan_is_signed_in_and_invalid_output_is_unavailable() {
        let status = parse_status(br#"{"loggedIn":true,"email":"fixture@example.test","orgName":"Fixture","subscriptionType":null,"configDirectory":"/tmp/fixture"}"#);
        assert!(status.logged_in);
        assert_eq!(status.plan, None);
        assert_eq!(status.organization.as_deref(), Some("Fixture"));
        assert!(!status.error);
        assert!(parse_status(b"invalid JSON").error);
        assert!(!parse_status(br#"{"loggedIn":false}"#).logged_in);
    }

    #[test]
    fn account_json_uses_app_state_without_storing_authentication() {
        let pool = db::open_in_memory().unwrap();
        let home = tempfile::tempdir().unwrap();
        let conn = pool.get().unwrap();
        let original = load_at(&conn, home.path()).unwrap();
        assert_eq!(original.accounts.len(), 1);
        assert_eq!(db::app_state_get(&conn, KEY).unwrap(), None);
        let json = serde_json::to_string(&original).unwrap();
        assert!(!json.contains("logged_in") && !json.contains("email") && !json.contains("token"));
        db::app_state_set(&conn, KEY, &json).unwrap();
        assert_eq!(
            load_at(&conn, home.path()).unwrap().accounts,
            original.accounts
        );
    }

    #[test]
    fn resume_rule_preserves_id_or_uses_origin_path_and_last_permission() {
        let home = tempfile::tempdir().unwrap();
        let origin = home
            .path()
            .join("origin folder")
            .to_string_lossy()
            .into_owned();
        let key = uuid::Uuid::new_v4().to_string();
        let path = transcript(&origin, Some("C:\\folder with spaces\\project"), &key).unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            "{\"permissionMode\":\"default\"}\n{\"permissionMode\":\"acceptEdits\"}\ninvalid\n",
        )
        .unwrap();
        let mut account = ClaudeSessionAccount {
            account_id: None,
            folder: origin.clone(),
            native: false,
            pinned: false,
            origin_folder: origin.clone(),
            permission_mode: last_permission(
                &origin,
                Some("C:\\folder with spaces\\project"),
                &key,
                "default",
            ),
        };
        let old = vec!["--resume".into(), key.clone()];
        let mut args = old.clone();
        resume_args(
            &mut args,
            &account,
            &origin,
            Some("C:\\folder with spaces\\project"),
            &key,
        );
        assert_eq!(args, old);
        account.folder = home.path().join("active").to_string_lossy().into_owned();
        resume_args(
            &mut args,
            &account,
            &origin,
            Some("C:\\folder with spaces\\project"),
            &key,
        );
        assert_eq!(
            args,
            vec![
                "--resume".to_owned(),
                path.to_string_lossy().into_owned(),
                "--permission-mode".into(),
                "acceptEdits".into()
            ]
        );
        assert!(transcript(&origin, Some("/tmp"), "../../escape").is_none());
    }
}
