use super::{AccountsState, ClaudeAccount, ClaudeAccountAction};
use crate::{
    db,
    error::{Error, Result},
    model::Role,
    session::runtime::SpawnSpec,
};
use runner_core::protocol::{ClaudeAccountSharing, ClaudeSessionAccount};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
};

const ITEMS: &[&str] = &[
    "CLAUDE.md",
    "skills",
    "agents",
    "commands",
    "plugins",
    "settings.json",
];
pub(crate) const MAIN_FOLDER: &str = "RUNNER_CLAUDE_MAIN_FOLDER";
pub(crate) const MAIN_NATIVE: &str = "RUNNER_CLAUDE_MAIN_NATIVE";
pub(crate) const MAIN_MCP: &str = "RUNNER_CLAUDE_MAIN_MCP";

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct SharedConfig {
    main_folder: PathBuf,
    folder: PathBuf,
    links: Vec<OwnedLink>,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct MainChange {
    pub target_id: String,
    linked_ids: Vec<String>,
    retargeting: bool,
}

pub(super) fn validate_action(state: &AccountsState, action: &ClaudeAccountAction) -> Result<()> {
    let Some(change) = &state.main_change else {
        return Ok(());
    };
    let allowed = matches!(
        action,
        ClaudeAccountAction::Rename { .. } | ClaudeAccountAction::Use { .. }
    ) || matches!(action, ClaudeAccountAction::MakeMain {id} if *id == change.target_id);
    if allowed {
        Ok(())
    } else {
        Err(Error::msg(
            "main account change is incomplete; retry Make main in Settings",
        ))
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct OwnedLink {
    name: String,
    source: PathBuf,
    import: bool,
}

fn canonical(path: &Path) -> io::Result<PathBuf> {
    let path = fs::canonicalize(path)?;
    #[cfg(windows)]
    let path = crate::runtimes::ordinary_windows_path(&path);
    Ok(path)
}

fn exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

fn import_text(source: &Path) -> String {
    format!(
        "@{}\n",
        source
            .to_string_lossy()
            .replace('\\', "/")
            .replace(' ', "\\ ")
    )
}

fn link_target(path: &Path) -> io::Result<PathBuf> {
    #[cfg(windows)]
    {
        junction::get_target(path)
            .or_else(|_| fs::read_link(path))
            .map(|path| crate::runtimes::ordinary_windows_path(&path))
    }
    #[cfg(not(windows))]
    fs::read_link(path)
}

fn skill_name(name: &str) -> Option<&str> {
    name.strip_prefix("skills/").filter(|name| {
        !name.is_empty()
            && !name.starts_with('.')
            && *name != "synced"
            && !name.contains(['/', '\\'])
    })
}

fn is_link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    metadata.file_type().is_symlink()
}

fn owned(config: &SharedConfig, link: &OwnedLink) -> io::Result<bool> {
    if (!ITEMS.contains(&link.name.as_str()) && skill_name(&link.name).is_none())
        || link.source != config.main_folder.join(&link.name)
    {
        return Ok(false);
    }
    match canonical(&config.folder) {
        Ok(folder) if folder == config.folder => {}
        Ok(_) => return Ok(false),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    }
    if skill_name(&link.name).is_some() {
        match fs::symlink_metadata(config.folder.join("skills")) {
            Ok(metadata) if metadata.is_dir() && !is_link(&metadata) => {}
            Ok(_) => return Ok(false),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error),
        }
    }
    let path = config.folder.join(&link.name);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    if link.import {
        if !metadata.file_type().is_file() {
            return Ok(false);
        }
        return Ok(fs::read(path)? == import_text(&link.source).as_bytes());
    }
    if !is_link(&metadata) {
        return Ok(false);
    }
    match link_target(&path) {
        Ok(target) => Ok(target == link.source),
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::InvalidInput
            ) =>
        {
            Ok(false)
        }
        Err(error) => Err(error),
    }
}

fn unlink_item(config: &SharedConfig, link: &OwnedLink) -> Result<()> {
    if owned(config, link)? {
        let path = config.folder.join(&link.name);
        #[cfg(windows)]
        if !link.import && junction::get_target(&path).is_ok() {
            fs::remove_dir(&path)?;
        } else {
            fs::remove_file(&path)?;
        }
        #[cfg(not(windows))]
        fs::remove_file(path)?;
    }
    Ok(())
}

fn unlink(config: &mut SharedConfig) -> Result<()> {
    while let Some(link) = config.links.last() {
        unlink_item(config, link)?;
        config.links.pop();
    }
    Ok(())
}

fn add_link(config: &mut SharedConfig, name: &str) -> Result<()> {
    let path = config.folder.join(name);
    if exists(&path) {
        return Ok(());
    }
    let source = config.main_folder.join(name);
    #[cfg(unix)]
    std::os::unix::fs::symlink(&source, &path)?;
    #[cfg(windows)]
    if name == "CLAUDE.md" {
        use std::io::Write;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        file.write_all(import_text(&source).as_bytes())?;
    } else if name == "settings.json" {
        const ERROR_PRIVILEGE_NOT_HELD: i32 = 1314;
        match std::os::windows::fs::symlink_file(&source, path) {
            Ok(()) => {}
            // Without symbolic-link permission, `launch` passes main settings as flag settings.
            Err(error) if error.raw_os_error() == Some(ERROR_PRIVILEGE_NOT_HELD) => return Ok(()),
            Err(error) => return Err(error.into()),
        }
    } else {
        junction::create(&source, path)?;
    }
    config.links.retain(|link| link.name != name);
    config.links.push(OwnedLink {
        name: name.into(),
        source,
        import: cfg!(windows) && name == "CLAUDE.md",
    });
    Ok(())
}

fn share(config: &mut SharedConfig) -> Result<()> {
    for index in (0..config.links.len()).rev() {
        if matches!(config.links[index].name.as_str(), "skills" | "plugins") {
            unlink_item(config, &config.links[index])?;
            config.links.remove(index);
        }
    }
    for name in ["CLAUDE.md", "agents", "commands", "settings.json"] {
        add_link(config, name)?;
    }
    let skills = config.folder.join("skills");
    match fs::symlink_metadata(&skills) {
        Ok(metadata) if metadata.is_dir() && !is_link(&metadata) => {}
        Ok(_) => return Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => fs::create_dir(&skills)?,
        Err(error) => return Err(error.into()),
    }
    let entries = match fs::read_dir(config.main_folder.join("skills")) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    for entry in entries {
        let entry = entry?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let name = format!("skills/{name}");
        if skill_name(&name).is_some() && entry.path().is_dir() {
            add_link(config, &name)?;
        }
    }
    Ok(())
}

fn stop(state: &mut AccountsState, id: &str) -> Result<()> {
    if let Some(config) = state.sharing.get_mut(id) {
        unlink(config)?;
    }
    state.sharing.remove(id);
    Ok(())
}

fn start(state: &mut AccountsState, id: &str) -> Result<()> {
    start_for(state, id, &state.main_id.clone())
}

fn start_for(state: &mut AccountsState, id: &str, main_id: &str) -> Result<()> {
    if id == main_id {
        return Err(Error::msg("the main account already owns these settings"));
    }
    let account = state
        .accounts
        .iter()
        .find(|account| account.id == id)
        .ok_or_else(|| Error::msg("account not found"))?;
    let main = state
        .accounts
        .iter()
        .find(|account| account.id == main_id)
        .ok_or_else(|| Error::msg("account not found"))?;
    fs::create_dir_all(&account.folder)?;
    fs::create_dir_all(&main.folder)?;
    let folder = canonical(Path::new(&account.folder))?;
    let main_folder = canonical(Path::new(&main.folder))?;
    if folder == main_folder {
        return Err(Error::msg("account folders resolve to the same location"));
    }
    if let Some(config) = state.sharing.get_mut(id) {
        if config.main_folder != main_folder || config.folder != folder {
            unlink(config)?;
        }
    }
    let config = state
        .sharing
        .entry(id.into())
        .or_insert_with(|| SharedConfig {
            main_folder: main_folder.clone(),
            folder: folder.clone(),
            links: Vec::new(),
        });
    config.main_folder = main_folder;
    config.folder = folder;
    share(config)
}

pub(super) fn apply_action(
    state: &mut AccountsState,
    previous: &AccountsState,
    action: &ClaudeAccountAction,
    created: Option<&str>,
) -> Result<()> {
    match action {
        ClaudeAccountAction::Add {
            use_main_settings: true,
            ..
        }
        | ClaudeAccountAction::AddAndLogin {
            use_main_settings: true,
            ..
        } => start(state, created.unwrap()),
        ClaudeAccountAction::Share { id } => start(state, id),
        ClaudeAccountAction::StopSharing { id } | ClaudeAccountAction::Remove { id } => {
            stop(state, id)
        }
        ClaudeAccountAction::MakeMain { id } => {
            state.main_id.clone_from(&previous.main_id);
            change_main(state, id)
        }
        _ => Ok(()),
    }
}

fn change_main(state: &mut AccountsState, target: &str) -> Result<()> {
    if state.main_change.is_none() && state.main_id == target {
        return Ok(());
    }
    let account = state
        .accounts
        .iter()
        .find(|account| account.id == target)
        .ok_or_else(|| Error::msg("account not found"))?;
    fs::create_dir_all(&account.folder)?;
    if state.main_change.is_none() {
        state.main_change = Some(MainChange {
            target_id: target.into(),
            linked_ids: state.sharing.keys().cloned().collect(),
            retargeting: false,
        });
    }
    let change = state.main_change.as_ref().unwrap().clone();
    if change.target_id != target {
        return Err(Error::msg(
            "main account change is incomplete; retry Make main in Settings",
        ));
    }
    if !change.retargeting {
        // Keep the old owner authoritative until every old managed link is gone.
        for id in &change.linked_ids {
            if let Some(config) = state.sharing.get_mut(id) {
                unlink(config)?;
            }
        }
        state.main_change.as_mut().unwrap().retargeting = true;
    }
    for id in &change.linked_ids {
        if id != target {
            start_for(state, id, target)?;
        }
    }
    state.sharing.remove(target);
    state.main_id = target.into();
    state.main_change = None;
    Ok(())
}

pub(super) fn summaries(state: &AccountsState) -> BTreeMap<String, ClaudeAccountSharing> {
    state
        .sharing
        .iter()
        .filter_map(|(id, config)| {
            let account = state.accounts.iter().find(|account| &account.id == id)?;
            let own_items = ITEMS
                .iter()
                .copied()
                .filter(|name| {
                    exists(&Path::new(&account.folder).join(name))
                        && !config
                            .links
                            .iter()
                            .any(|link| link.name == *name && owned(config, link).unwrap_or(false))
                })
                .map(str::to_owned)
                .collect();
            Some((
                id.clone(),
                ClaudeAccountSharing {
                    using_main: state.main_change.is_none(),
                    shared_skills: config
                        .links
                        .iter()
                        .filter(|link| {
                            skill_name(&link.name).is_some() && owned(config, link).unwrap_or(false)
                        })
                        .count(),
                    shared_plugins: plugin_dirs(&config.main_folder)
                        .map(|dirs| dirs.len())
                        .unwrap_or(0),
                    own_items,
                },
            ))
        })
        .collect()
}

pub(crate) fn main_account(pool: &db::DbPool) -> Option<ClaudeAccount> {
    let state = super::load(pool).ok()?;
    (state.accounts.len() > 1)
        .then(|| {
            state
                .accounts
                .into_iter()
                .find(|account| account.id == state.main_id)
        })
        .flatten()
}

pub(crate) fn main_channel(pool: &db::DbPool) -> Option<&'static str> {
    main_account(pool).map(|main| {
        crate::runtimes::claude_code::claude_channel(&Path::new(&main.folder).join("settings.json"))
            .unwrap_or("latest")
    })
}

pub(crate) fn mcp_path(account: &ClaudeAccount) -> PathBuf {
    if account.native {
        Path::new(&account.folder)
            .parent()
            .unwrap()
            .join(".claude.json")
    } else {
        Path::new(&account.folder).join(".claude.json")
    }
}

pub(crate) fn config_env(
    pool: &db::DbPool,
    mut env: BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    if let Some(main) = main_account(pool) {
        env.insert(MAIN_NATIVE.into(), main.native.to_string());
        env.insert(MAIN_FOLDER.into(), main.folder.clone());
        env.insert(
            MAIN_MCP.into(),
            mcp_path(&main).to_string_lossy().into_owned(),
        );
    }
    env
}

pub(crate) fn account_config_env(
    pool: &db::DbPool,
    env: BTreeMap<String, String>,
    id: Option<&str>,
) -> Result<BTreeMap<String, String>> {
    let Some(id) = id else {
        return Ok(config_env(pool, env));
    };
    let state = super::load(pool)?;
    let account = state
        .accounts
        .iter()
        .find(|account| account.id == id)
        .ok_or_else(|| Error::msg("unknown Claude Code account"))?;
    let account = if state.sharing.contains_key(id) {
        state
            .accounts
            .iter()
            .find(|account| account.id == state.main_id)
            .ok_or_else(|| Error::msg("missing main Claude Code account"))?
    } else {
        account
    };
    let mut env = env;
    env.insert(MAIN_FOLDER.into(), account.folder.clone());
    env.insert(
        MAIN_MCP.into(),
        mcp_path(account).to_string_lossy().into_owned(),
    );
    env.insert(MAIN_NATIVE.into(), account.native.to_string());
    Ok(env)
}

fn object(path: &Path) -> Result<Value> {
    let value = match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => json!({}),
        Err(error) => return Err(error.into()),
    };
    if !value.is_object() {
        return Err(Error::msg("Claude configuration is not a JSON object"));
    }
    Ok(value)
}

fn plugin_dirs(main: &Path) -> Result<Vec<PathBuf>> {
    let settings = object(&main.join("settings.json"))?;
    let installed = object(&main.join("plugins/installed_plugins.json"))?;
    let Some(enabled) = settings.get("enabledPlugins").and_then(Value::as_object) else {
        return Ok(Vec::new());
    };
    let mut dirs = Vec::new();
    for (name, enabled) in enabled {
        if enabled != &Value::Bool(true) {
            continue;
        }
        let Some(entries) = installed
            .get("plugins")
            .and_then(|plugins| plugins.get(name))
            .and_then(Value::as_array)
        else {
            continue;
        };
        if let Some(path) = entries
            .iter()
            .find(|entry| entry.get("scope").and_then(Value::as_str) == Some("user"))
            .and_then(|entry| entry.get("installPath"))
            .and_then(Value::as_str)
            .map(Path::new)
            .filter(|path| path.is_dir())
        {
            dirs.push(canonical(path)?);
        }
    }
    dirs.sort();
    dirs.dedup();
    Ok(dirs)
}

fn merge(base: &mut Value, overlay: Value) {
    if let (Some(base), Some(overlay)) = (base.as_object_mut(), overlay.as_object()) {
        for (key, value) in overlay {
            match base.get_mut(key) {
                Some(old) if old.is_object() && value.is_object() => merge(old, value.clone()),
                Some(Value::Array(old)) if value.is_array() => {
                    old.extend(value.as_array().unwrap().iter().cloned())
                }
                _ => {
                    base.insert(key.clone(), value.clone());
                }
            }
        }
    }
}

fn merge_main_settings(main: &Path, spec: &mut SpawnSpec) -> Result<()> {
    if let Some(index) = spec.args.iter().position(|arg| arg == "--settings") {
        let mut settings = object(&main.join("settings.json"))?;
        let runner: Value = serde_json::from_str(&spec.args[index + 1])?;
        merge(&mut settings, runner);
        spec.args[index + 1] = serde_json::to_string(&settings)?;
    }
    Ok(())
}

pub(crate) fn launch(
    pool: &db::DbPool,
    account: Option<&ClaudeSessionAccount>,
    role: &Role,
    spec: &mut SpawnSpec,
) -> Result<()> {
    let Some(account) = account else {
        return Ok(());
    };
    let mut conn = pool.get()?;
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let home = runner_core::app_paths::home_dir()
        .ok_or_else(|| Error::msg("home directory unavailable"))?;
    let mut state = super::load_at(&tx, &home)?;
    let Some(target) = state
        .accounts
        .iter()
        .find(|target| {
            target.native == account.native && (target.native || target.folder == account.folder)
        })
        .cloned()
    else {
        return Ok(());
    };
    if state
        .main_change
        .as_ref()
        .is_some_and(|change| change.linked_ids.contains(&target.id))
    {
        return Err(Error::msg(
            "main account change is incomplete; retry Make main in Settings",
        ));
    }
    let Some(config) = state.sharing.get_mut(&target.id) else {
        return Ok(());
    };
    let sharing_result = share(config);
    db::app_state_set(&tx, super::KEY, &serde_json::to_string(&state)?)?;
    tx.commit()?;
    drop(conn);
    sharing_result?;
    let main = state
        .accounts
        .iter()
        .find(|main| main.id == state.main_id)
        .ok_or_else(|| Error::msg("account not found"))?;
    let own_settings = Path::new(&target.folder).join("settings.json");
    let custom = role
        .args
        .iter()
        .any(|arg| arg == "--settings" || arg.starts_with("--settings="));
    if !exists(&own_settings) && !custom {
        merge_main_settings(Path::new(&main.folder), spec)?;
    }
    let plugin_args = plugin_dirs(Path::new(&main.folder))?
        .into_iter()
        .flat_map(|path| {
            [
                "--plugin-dir".to_owned(),
                path.to_string_lossy().into_owned(),
            ]
        });
    spec.args.splice(0..0, plugin_args);
    let identity = object(&mcp_path(main))?;
    if let Some(servers) = identity
        .get("mcpServers")
        .filter(|servers| servers.is_object())
    {
        if !servers.as_object().unwrap().is_empty() {
            spec.args.splice(
                0..0,
                [
                    "--mcp-config".into(),
                    serde_json::to_string(&json!({"mcpServers": servers}))?,
                ],
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{model::Runtime, shell_path::LoginShellEnv};

    fn fixture() -> (tempfile::TempDir, AccountsState) {
        let temp = tempfile::tempdir().unwrap();
        let mut state = super::super::initial(temp.path());
        let main = Path::new(&state.accounts[0].folder);
        fs::create_dir_all(main).unwrap();
        for name in ["skills", "agents", "commands", "plugins"] {
            fs::create_dir(main.join(name)).unwrap();
        }
        fs::write(main.join("CLAUDE.md"), "source memory").unwrap();
        fs::write(main.join("settings.json"), r#"{"theme":"dark","env":{"SHARED":"yes"},"enabledPlugins":{"demo@fixture":true},"hooks":{"Stop":[{"hooks":[{"command":"main-hook","type":"command"}]}]}}"#).unwrap();
        fs::write(
            temp.path().join(".claude.json"),
            r#"{"identity":"NEVER-IN-ARGV","mcpServers":{"fixture":{"command":"fixture-tool"}}}"#,
        )
        .unwrap();
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
        (temp, state)
    }

    fn save(state: &AccountsState) -> db::DbPool {
        let pool = db::open_in_memory().unwrap();
        db::app_state_set(
            &pool.get().unwrap(),
            super::super::KEY,
            &serde_json::to_string(state).unwrap(),
        )
        .unwrap();
        pool
    }

    fn session(state: &AccountsState) -> ClaudeSessionAccount {
        let target = &state.accounts[1];
        ClaudeSessionAccount {
            account_id: Some(target.id.clone()),
            folder: target.folder.clone(),
            native: false,
            pinned: false,
            origin_folder: target.folder.clone(),
            permission_mode: "default".into(),
        }
    }

    fn role() -> Role {
        crate::session::manager::runtime_direct_role("claude-code", Some("claude"), None, None)
            .unwrap()
    }

    fn spec() -> SpawnSpec {
        SpawnSpec {args:vec!["--settings".into(),json!({"tui":{"fullscreen":true},"theme":"auto","hooks":{"Stop":[{"hooks":[{"command":"runner-hook","type":"command"}]}]}}).to_string(),"tiny prompt".into()],..Default::default()}
    }

    fn write_skill(folder: &Path, name: &str, text: &str) -> PathBuf {
        let path = folder.join("skills").join(name);
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("SKILL.md"), text).unwrap();
        path
    }

    fn directory_link(source: &Path, path: &Path) {
        #[cfg(unix)]
        std::os::unix::fs::symlink(source, path).unwrap();
        #[cfg(windows)]
        junction::create(source, path).unwrap();
    }

    fn remove_directory_link(path: &Path) {
        #[cfg(unix)]
        fs::remove_file(path).unwrap();
        #[cfg(windows)]
        fs::remove_dir(path).unwrap();
    }

    #[test]
    fn catalog_account_scope_reads_own_folder_and_sharing_reads_main() {
        let (temp, mut state) = fixture();
        let main = PathBuf::from(&state.accounts[0].folder);
        let own = PathBuf::from(&state.accounts[1].folder);
        for (folder, name) in [(&main, "main-skill"), (&own, "own-skill")] {
            fs::create_dir_all(folder.join("skills").join(name)).unwrap();
            fs::write(
                folder.join("skills").join(name).join("SKILL.md"),
                format!("---\nname: {name}\n---\n# Skill"),
            )
            .unwrap();
        }
        let pool = save(&state);
        let env = account_config_env(&pool, Default::default(), Some("work")).unwrap();
        let catalog =
            crate::skills::skill_catalog_with_env(Runtime::ClaudeCode, temp.path(), None, &env)
                .unwrap();
        assert_eq!(catalog.roots, [own.join("skills")]);
        assert_eq!(catalog.entries[0].name, "own-skill");
        crate::runtimes::adapter(Runtime::ClaudeCode)
            .toggle_skill_with_env(temp.path(), &catalog.entries[0].path, false, &env)
            .unwrap()
            .unwrap();
        assert!(fs::read_to_string(own.join("settings.json"))
            .unwrap()
            .contains("own-skill"));
        assert!(!fs::read_to_string(main.join("settings.json"))
            .unwrap()
            .contains("own-skill"));
        start(&mut state, "work").unwrap();
        let pool = save(&state);
        let env = account_config_env(&pool, Default::default(), Some("work")).unwrap();
        let catalog =
            crate::skills::skill_catalog_with_env(Runtime::ClaudeCode, temp.path(), None, &env)
                .unwrap();
        assert_eq!(catalog.roots, [main.join("skills")]);
        assert_eq!(catalog.entries[0].name, "main-skill");
        assert!(account_config_env(&pool, Default::default(), Some("missing")).is_err());
        assert_eq!(
            account_config_env(&pool, Default::default(), None).unwrap(),
            config_env(&pool, Default::default())
        );
    }

    #[test]
    fn skill_entries_preserve_synced_conflicts_and_local_plugin_state() {
        let (_temp, mut state) = fixture();
        let main = PathBuf::from(&state.accounts[0].folder);
        let folder = PathBuf::from(&state.accounts[1].folder);
        for name in ["shared955", "conflict", "synced", ".trash", ".hidden"] {
            write_skill(&main, name, "main skill");
        }
        for name in ["conflict", "synced", ".trash", "work-only"] {
            write_skill(&folder, name, "account skill");
        }
        fs::create_dir(folder.join("plugins")).unwrap();
        fs::write(
            folder.join("plugins/installed_plugins.json"),
            "account plugin state",
        )
        .unwrap();
        start(&mut state, "work").unwrap();
        assert!(!is_link(
            &fs::symlink_metadata(folder.join("skills")).unwrap()
        ));
        assert!(!is_link(
            &fs::symlink_metadata(folder.join("plugins")).unwrap()
        ));
        assert_eq!(
            fs::read_to_string(folder.join("skills/shared955/SKILL.md")).unwrap(),
            "main skill"
        );
        assert!(!exists(&folder.join("skills/.hidden")));
        assert_eq!(summaries(&state)["work"].shared_skills, 1);
        let mut restored = super::super::load(&save(&state)).unwrap();
        stop(&mut restored, "work").unwrap();
        assert!(!exists(&folder.join("skills/shared955")));
        for name in ["conflict", "synced", ".trash", "work-only"] {
            assert_eq!(
                fs::read_to_string(folder.join("skills").join(name).join("SKILL.md")).unwrap(),
                "account skill"
            );
        }
        assert_eq!(
            fs::read_to_string(folder.join("plugins/installed_plugins.json")).unwrap(),
            "account plugin state"
        );
        assert_eq!(
            fs::read_to_string(main.join("skills/shared955/SKILL.md")).unwrap(),
            "main skill"
        );
    }

    #[test]
    fn launch_recreates_deleted_skill_with_one_persisted_owner() {
        let (temp, mut state) = fixture();
        let main = PathBuf::from(&state.accounts[0].folder);
        let folder = PathBuf::from(&state.accounts[1].folder);
        write_skill(&main, "shared955", "main skill stays");
        write_skill(&folder, "own-skill", "own skill stays");
        write_skill(&folder, "synced", "synced skill stays");
        start(&mut state, "work").unwrap();
        let mut core = crate::test_support::test_core_in(temp.path().join("core"));
        core.usage = std::sync::Arc::new(crate::usage::UsageService::without_fetch());
        core.claude_accounts =
            std::sync::Arc::new(super::super::AccountService::without_authentication());
        db::app_state_set(
            &core.db.get().unwrap(),
            super::super::KEY,
            &serde_json::to_string(&state).unwrap(),
        )
        .unwrap();
        for _ in 0..3 {
            remove_directory_link(&folder.join("skills/shared955"));
            launch(&core.db, Some(&session(&state)), &role(), &mut spec()).unwrap();
            state = super::super::load(&core.db).unwrap();
            let records = state.sharing["work"]
                .links
                .iter()
                .filter(|link| link.name == "skills/shared955")
                .count();
            let snapshot = super::super::snapshot(&core).unwrap();
            assert_eq!((records, snapshot.sharing["work"].shared_skills), (1, 1));
            assert_eq!(
                fs::read_to_string(folder.join("skills/shared955/SKILL.md")).unwrap(),
                "main skill stays"
            );
        }
        let stopped = super::super::action(
            &core,
            ClaudeAccountAction::StopSharing { id: "work".into() },
        )
        .unwrap();
        assert!(!stopped.sharing.contains_key("work"));
        assert!(!super::super::load(&core.db)
            .unwrap()
            .sharing
            .contains_key("work"));
        assert!(!exists(&folder.join("skills/shared955")));
        assert!(!is_link(
            &fs::symlink_metadata(folder.join("skills")).unwrap()
        ));
        assert_eq!(
            fs::read_to_string(folder.join("skills/own-skill/SKILL.md")).unwrap(),
            "own skill stays"
        );
        assert_eq!(
            fs::read_to_string(folder.join("skills/synced/SKILL.md")).unwrap(),
            "synced skill stays"
        );
        assert_eq!(
            fs::read_to_string(main.join("skills/shared955/SKILL.md")).unwrap(),
            "main skill stays"
        );
    }

    #[test]
    fn skill_entries_retarget_and_preserve_user_replacements() {
        let (temp, mut state) = fixture();
        let main = PathBuf::from(&state.accounts[0].folder);
        let work = PathBuf::from(&state.accounts[1].folder);
        let other = temp.path().join("other folder");
        write_skill(&main, "native-skill", "native stays");
        write_skill(&work, "work-skill", "work owns");
        state.accounts.push(ClaudeAccount {
            id: "other".into(),
            name: "Other".into(),
            folder: other.to_string_lossy().into_owned(),
            native: false,
        });
        start(&mut state, "work").unwrap();
        start(&mut state, "other").unwrap();
        let replaced = other.join("skills/native-skill");
        remove_directory_link(&replaced);
        write_skill(&other, "native-skill", "user replacement");
        change_main(&mut state, "work").unwrap();
        assert!(!exists(&work.join("skills/native-skill")));
        assert_eq!(
            fs::read_to_string(other.join("skills/work-skill/SKILL.md")).unwrap(),
            "work owns"
        );
        assert!(state.sharing["other"]
            .links
            .iter()
            .any(|link| link.name == "skills/work-skill"
                && owned(&state.sharing["other"], link).unwrap()));
        stop(&mut state, "other").unwrap();
        assert!(!exists(&other.join("skills/work-skill")));
        assert_eq!(
            fs::read_to_string(replaced.join("SKILL.md")).unwrap(),
            "user replacement"
        );
        assert_eq!(
            fs::read_to_string(main.join("skills/native-skill/SKILL.md")).unwrap(),
            "native stays"
        );
    }

    #[test]
    fn legacy_skill_and_plugin_parent_links_are_retired_safely() {
        let (_temp, mut state) = fixture();
        let main = PathBuf::from(&state.accounts[0].folder);
        let folder = PathBuf::from(&state.accounts[1].folder);
        write_skill(&main, "shared955", "source stays");
        fs::write(main.join("plugins/canary"), "plugin source stays").unwrap();
        fs::create_dir(&folder).unwrap();
        let mut config = SharedConfig {
            main_folder: canonical(&main).unwrap(),
            folder: canonical(&folder).unwrap(),
            links: Vec::new(),
        };
        add_link(&mut config, "skills").unwrap();
        add_link(&mut config, "plugins").unwrap();
        state.sharing.insert("work".into(), config);
        start(&mut state, "work").unwrap();
        assert!(!is_link(
            &fs::symlink_metadata(folder.join("skills")).unwrap()
        ));
        assert!(!exists(&folder.join("plugins")));
        assert_eq!(
            fs::read_to_string(folder.join("skills/shared955/SKILL.md")).unwrap(),
            "source stays"
        );
        assert!(state.sharing["work"]
            .links
            .iter()
            .all(|link| !matches!(link.name.as_str(), "skills" | "plugins")));
        stop(&mut state, "work").unwrap();
        assert!(folder.join("skills").is_dir());
        assert_eq!(
            fs::read_to_string(main.join("skills/shared955/SKILL.md")).unwrap(),
            "source stays"
        );
        assert_eq!(
            fs::read_to_string(main.join("plugins/canary")).unwrap(),
            "plugin source stays"
        );
    }

    #[test]
    fn foreign_skill_parent_links_are_never_populated_or_cleaned() {
        let (temp, mut state) = fixture();
        let main = PathBuf::from(&state.accounts[0].folder);
        let folder = PathBuf::from(&state.accounts[1].folder);
        let source = write_skill(&main, "shared955", "main skill");
        let foreign = temp.path().join("foreign skills");
        fs::create_dir(&foreign).unwrap();
        fs::create_dir(&folder).unwrap();
        directory_link(&foreign, &folder.join("skills"));
        start(&mut state, "work").unwrap();
        assert!(!exists(&foreign.join("shared955")));
        remove_directory_link(&folder.join("skills"));
        start(&mut state, "work").unwrap();
        remove_directory_link(&folder.join("skills/shared955"));
        fs::remove_dir(folder.join("skills")).unwrap();
        directory_link(&source, &foreign.join("shared955"));
        directory_link(&foreign, &folder.join("skills"));
        stop(&mut state, "work").unwrap();
        assert!(is_link(
            &fs::symlink_metadata(folder.join("skills")).unwrap()
        ));
        assert!(is_link(
            &fs::symlink_metadata(foreign.join("shared955")).unwrap()
        ));
        assert_eq!(
            fs::read_to_string(foreign.join("shared955/SKILL.md")).unwrap(),
            "main skill"
        );
    }

    #[test]
    fn plugin_dirs_load_only_enabled_user_installs_without_linking_state() {
        let (_temp, mut state) = fixture();
        let main = PathBuf::from(&state.accounts[0].folder);
        let folder = PathBuf::from(&state.accounts[1].folder);
        let first = main.join("plugins/cache/first plugin");
        let second = main.join("plugins/cache/second plugin");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();
        let settings=json!({"enabledPlugins":{"first@fixture":true,"second@fixture":true,"disabled@fixture":false,"missing@fixture":true,"project@fixture":true}}).to_string();
        let installed = json!({"version":2,"plugins":{
            "first@fixture":[{"scope":"user","installPath":first}],
            "second@fixture":[{"scope":"user","installPath":second}],
            "disabled@fixture":[{"scope":"user","installPath":first}],
            "missing@fixture":[{"scope":"user","installPath":main.join("not installed")}],
            "project@fixture":[{"scope":"project","installPath":first}]
        }})
        .to_string();
        fs::write(main.join("settings.json"), &settings).unwrap();
        fs::write(main.join("plugins/installed_plugins.json"), &installed).unwrap();
        fs::create_dir_all(folder.join("plugins")).unwrap();
        fs::write(folder.join("plugins/canary"), "account plugin state").unwrap();
        start(&mut state, "work").unwrap();
        assert_eq!(summaries(&state)["work"].shared_plugins, 2);
        let pool = save(&state);
        let mut launch_spec = spec();
        launch(&pool, Some(&session(&state)), &role(), &mut launch_spec).unwrap();
        let dirs: Vec<_> = launch_spec
            .args
            .windows(2)
            .filter(|pair| pair[0] == "--plugin-dir")
            .map(|pair| PathBuf::from(&pair[1]))
            .collect();
        assert_eq!(
            dirs,
            vec![canonical(&first).unwrap(), canonical(&second).unwrap()]
        );
        assert_eq!(launch_spec.args.last().unwrap(), "tiny prompt");
        assert_eq!(
            fs::read_to_string(main.join("settings.json")).unwrap(),
            settings
        );
        assert_eq!(
            fs::read_to_string(main.join("plugins/installed_plugins.json")).unwrap(),
            installed
        );
        assert_eq!(
            fs::read_to_string(folder.join("plugins/canary")).unwrap(),
            "account plugin state"
        );
        assert!(!is_link(
            &fs::symlink_metadata(folder.join("plugins")).unwrap()
        ));
        stop(&mut state, "work").unwrap();
        let pool = save(&state);
        let mut independent = spec();
        let expected = independent.args.clone();
        launch(&pool, Some(&session(&state)), &role(), &mut independent).unwrap();
        assert_eq!(independent.args, expected);
    }

    #[test]
    fn sharing_preserves_existing_files_and_unlinks_only_owned_items() {
        let (_temp, mut state) = fixture();
        let folder = PathBuf::from(&state.accounts[1].folder);
        fs::create_dir_all(folder.join("skills")).unwrap();
        fs::write(folder.join("skills/user-skill"), "keep").unwrap();
        fs::write(folder.join("settings.json"), "own settings exactly").unwrap();
        fs::write(folder.join(".claude.json"), "own identity exactly").unwrap();
        fs::write(folder.join(".credentials.json"), "own credentials exactly").unwrap();
        fs::create_dir(folder.join("projects")).unwrap();
        start(&mut state, "work").unwrap();
        #[cfg(unix)]
        let expected_memory = "source memory".to_owned();
        #[cfg(windows)]
        let expected_memory = import_text(
            &canonical(Path::new(&state.accounts[0].folder))
                .unwrap()
                .join("CLAUDE.md"),
        );
        assert_eq!(
            fs::read_to_string(folder.join("CLAUDE.md")).unwrap(),
            expected_memory
        );
        assert_eq!(
            summaries(&state)["work"].own_items,
            vec!["skills", "settings.json"]
        );
        let before = serde_json::to_string(&state.sharing).unwrap();
        start(&mut state, "work").unwrap();
        assert_eq!(before, serde_json::to_string(&state.sharing).unwrap());
        let mut restored: AccountsState =
            serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();
        stop(&mut restored, "work").unwrap();
        assert!(!exists(&folder.join("CLAUDE.md")));
        assert!(!exists(&folder.join("plugins")));
        for (file, expected) in [
            ("skills/user-skill", "keep"),
            ("settings.json", "own settings exactly"),
            (".claude.json", "own identity exactly"),
            (".credentials.json", "own credentials exactly"),
        ] {
            assert_eq!(fs::read_to_string(folder.join(file)).unwrap(), expected);
        }
        assert!(folder.join("projects").is_dir());
        assert_eq!(
            fs::read_to_string(Path::new(&state.accounts[0].folder).join("CLAUDE.md")).unwrap(),
            "source memory"
        );
    }

    #[test]
    fn stop_preserves_replaced_links_and_modified_imports() {
        let (_temp, mut state) = fixture();
        start(&mut state, "work").unwrap();
        let config = state.sharing["work"].clone();
        let memory = config
            .links
            .iter()
            .find(|link| link.name == "CLAUDE.md")
            .unwrap();
        let path = config.folder.join("CLAUDE.md");
        fs::remove_file(&path).unwrap();
        fs::write(&path, "user replaced this").unwrap();
        assert!(!owned(&config, memory).unwrap());
        let settings = config.folder.join("settings.json");
        fs::remove_file(&settings).unwrap();
        fs::write(&settings, "own settings").unwrap();
        stop(&mut state, "work").unwrap();
        assert_eq!(fs::read_to_string(path).unwrap(), "user replaced this");
        assert_eq!(fs::read_to_string(settings).unwrap(), "own settings");
        assert!(fs::read_to_string(config.main_folder.join("settings.json"))
            .unwrap()
            .contains("main-hook"));
    }

    #[cfg(unix)]
    #[test]
    fn existing_broken_and_retargeted_symlinks_are_never_claimed_or_removed() {
        let (temp, mut state) = fixture();
        let folder = PathBuf::from(&state.accounts[1].folder);
        fs::create_dir(&folder).unwrap();
        let user_target = temp.path().join("user missing target");
        std::os::unix::fs::symlink(&user_target, folder.join("agents")).unwrap();
        start(&mut state, "work").unwrap();
        let commands = folder.join("commands");
        fs::remove_file(&commands).unwrap();
        std::os::unix::fs::symlink(&user_target, &commands).unwrap();
        stop(&mut state, "work").unwrap();
        assert_eq!(fs::read_link(folder.join("agents")).unwrap(), user_target);
        assert_eq!(fs::read_link(commands).unwrap(), user_target);
    }

    #[test]
    fn missing_sources_can_appear_later_and_same_folder_sharing_is_rejected() {
        let (_temp, mut state) = fixture();
        fs::remove_dir(Path::new(&state.accounts[0].folder).join("skills")).unwrap();
        start(&mut state, "work").unwrap();
        let main = PathBuf::from(&state.accounts[0].folder);
        fs::create_dir(main.join("skills")).unwrap();
        fs::create_dir(main.join("skills/later")).unwrap();
        fs::write(main.join("skills/later/SKILL.md"), "visible").unwrap();
        let pool = save(&state);
        launch(&pool, Some(&session(&state)), &role(), &mut spec()).unwrap();
        state = super::super::load(&pool).unwrap();
        assert_eq!(
            fs::read_to_string(Path::new(&state.accounts[1].folder).join("skills/later/SKILL.md"))
                .unwrap(),
            "visible"
        );
        assert!(state.sharing["work"]
            .links
            .iter()
            .any(|link| link.name == "skills/later"));
        state.accounts[1].folder = state.accounts[0].folder.clone();
        assert!(start(&mut state, "work").is_err());
    }

    #[test]
    fn windows_memory_import_escapes_spaces_and_preserves_user_edits() {
        assert_eq!(
            import_text(Path::new("C:\\Account Folder\\CLAUDE.md")),
            "@C:/Account\\ Folder/CLAUDE.md\n"
        );
        let (_temp, mut state) = fixture();
        start(&mut state, "work").unwrap();
        let config = state.sharing.get_mut("work").unwrap();
        let link = config
            .links
            .iter_mut()
            .find(|link| link.name == "CLAUDE.md")
            .unwrap();
        let path = config.folder.join("CLAUDE.md");
        fs::remove_file(&path).unwrap();
        fs::write(&path, import_text(&link.source)).unwrap();
        link.import = true;
        assert!(owned(config, &config.links[0]).unwrap());
        fs::write(&path, "user memory").unwrap();
        stop(&mut state, "work").unwrap();
        assert_eq!(fs::read_to_string(path).unwrap(), "user memory");
    }

    #[cfg(windows)]
    #[test]
    fn junction_retargeting_preserves_the_replacement_and_both_targets() {
        let (temp, mut state) = fixture();
        let source = Path::new(&state.accounts[0].folder).join("skills/shared955");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("source-file"), "source stays").unwrap();
        start(&mut state, "work").unwrap();
        let path = Path::new(&state.accounts[1].folder).join("skills/shared955");
        assert!(junction::exists(&path).unwrap());
        fs::remove_dir(&path).unwrap();
        assert_eq!(
            fs::read_to_string(source.join("source-file")).unwrap(),
            "source stays"
        );
        let replacement = temp.path().join("replacement folder");
        fs::create_dir(&replacement).unwrap();
        fs::write(replacement.join("user-file"), "keep").unwrap();
        junction::create(&replacement, &path).unwrap();
        stop(&mut state, "work").unwrap();
        assert!(junction::exists(&path).unwrap());
        assert_eq!(fs::read_to_string(path.join("user-file")).unwrap(), "keep");
        assert!(source.is_dir());
        fs::remove_dir(path).unwrap();
    }

    #[test]
    fn launch_links_main_settings_and_adds_only_mcp_servers() {
        let (_temp, mut state) = fixture();
        start(&mut state, "work").unwrap();
        let main_settings = Path::new(&state.accounts[0].folder).join("settings.json");
        let settings = Path::new(&state.accounts[1].folder).join("settings.json");
        fs::remove_file(&settings).unwrap();
        let pool = save(&state);
        let mut spec = spec();
        let runner_settings = spec.args[1].clone();
        launch(&pool, Some(&session(&state)), &role(), &mut spec).unwrap();
        assert!(is_link(&fs::symlink_metadata(&settings).unwrap()));
        assert_eq!(
            fs::read_to_string(&settings).unwrap(),
            fs::read_to_string(&main_settings).unwrap()
        );
        assert_eq!(spec.args[2..4], ["--settings".to_owned(), runner_settings]);
        assert_eq!(spec.args[0], "--mcp-config");
        let mcp: Value = serde_json::from_str(&spec.args[1]).unwrap();
        assert_eq!(mcp.as_object().unwrap().len(), 1);
        assert!(mcp["mcpServers"]["fixture"].is_object());
        assert!(!spec.args.join(" ").contains("NEVER-IN-ARGV"));
        assert_eq!(spec.args.last().unwrap(), "tiny prompt");
        let mut state = super::super::load(&pool).unwrap();
        assert_eq!(summaries(&state)["work"].own_items, vec!["skills"]);
        stop(&mut state, "work").unwrap();
        assert!(!exists(&settings));
        assert!(main_settings.is_file());
    }

    #[test]
    fn main_settings_merge_into_one_settings_argument_without_a_link() {
        let (_temp, state) = fixture();
        let mut spec = spec();
        merge_main_settings(Path::new(&state.accounts[0].folder), &mut spec).unwrap();
        assert_eq!(
            spec.args.iter().filter(|arg| *arg == "--settings").count(),
            1
        );
        let settings: Value = serde_json::from_str(&spec.args[1]).unwrap();
        assert_eq!(settings["theme"], "auto");
        assert_eq!(settings["env"]["SHARED"], "yes");
        assert_eq!(settings["hooks"]["Stop"].as_array().unwrap().len(), 2);
        assert_eq!(settings["enabledPlugins"]["demo@fixture"], true);
        assert_eq!(spec.args.last().unwrap(), "tiny prompt");
    }

    #[test]
    fn own_settings_and_explicit_role_settings_override_the_shared_fallback() {
        let (_temp, mut state) = fixture();
        start(&mut state, "work").unwrap();
        let pool = save(&state);
        let settings = Path::new(&state.accounts[1].folder).join("settings.json");
        fs::remove_file(&settings).unwrap();
        fs::write(&settings, r#"{"own":true}"#).unwrap();
        let mut own = spec();
        let before = own.args[1].clone();
        launch(&pool, Some(&session(&state)), &role(), &mut own).unwrap();
        assert_eq!(own.args[3], before);
        fs::remove_file(settings).unwrap();
        let mut custom_role = role();
        custom_role.args = vec!["--settings=custom.json".into()];
        let mut custom = SpawnSpec {
            args: custom_role.args.clone(),
            ..Default::default()
        };
        launch(&pool, Some(&session(&state)), &custom_role, &mut custom).unwrap();
        assert_eq!(custom.args.last().unwrap(), "--settings=custom.json");
        assert_eq!(
            custom
                .args
                .iter()
                .filter(|arg| arg.starts_with("--settings"))
                .count(),
            1
        );
    }

    #[test]
    fn main_selection_moves_panes_model_identity_and_launch_mcp_without_native_identity() {
        let (temp, mut state) = fixture();
        start(&mut state, "work").unwrap();
        let previous = state.clone();
        state.main_id = "work".into();
        apply_action(
            &mut state,
            &previous,
            &ClaudeAccountAction::MakeMain { id: "work".into() },
            None,
        )
        .unwrap();
        assert!(state.sharing.is_empty());
        let folder = PathBuf::from(&state.accounts[1].folder);
        fs::create_dir_all(folder.join("skills/main-only")).unwrap();
        fs::write(
            folder.join("skills/main-only/SKILL.md"),
            "---\nname: main-only\n---\n",
        )
        .unwrap();
        fs::write(folder.join("settings.json"),r#"{"skillOverrides":{"main-only":"off"},"autoUpdatesChannel":"stable","model":"haiku"}"#).unwrap();
        fs::write(
            folder.join(".claude.json"),
            r#"{"mcpServers":{"work-only":{"command":"work-tool"}},"identity":"private"}"#,
        )
        .unwrap();
        let pool = save(&state);
        let env = config_env(&pool, Default::default());
        let catalog =
            crate::skills::skill_catalog_with_env(Runtime::ClaudeCode, temp.path(), None, &env)
                .unwrap();
        assert_eq!(catalog.roots, vec![folder.join("skills")]);
        assert_eq!(catalog.entries[0].global, crate::skills::GlobalState::Off);
        crate::runtimes::claude_code::skills::set_enabled_folder(
            &folder,
            catalog.entries[0].clone(),
            true,
        )
        .unwrap();
        assert!(!fs::read_to_string(folder.join("settings.json"))
            .unwrap()
            .contains("skillOverrides"));
        let mcp = crate::ops::mcp::mcp_catalog_with_env(temp.path(), &env).unwrap();
        assert!(format!("{mcp:?}").contains("work-only"));
        let source = crate::runtime_status::models::source_with_main(
            Runtime::ClaudeCode,
            "claude",
            &LoginShellEnv::default(),
            Some(&pool),
        );
        assert_eq!(
            serde_json::to_value(source).unwrap()["config_home"],
            folder.to_string_lossy().as_ref()
        );
        assert_eq!(
            crate::runtimes::claude_code::claude_channel(&folder.join("settings.json")),
            Some("stable")
        );
        assert!(state.accounts[0].native);
        assert_eq!(
            mcp_path(&state.accounts[0]),
            temp.path().join(".claude.json")
        );
    }

    #[test]
    fn unshared_unknown_pins_and_one_account_leave_launches_unchanged() {
        let (temp, mut state) = fixture();
        let pool = save(&state);
        let mut original = spec();
        let before = original.args.clone();
        launch(&pool, Some(&session(&state)), &role(), &mut original).unwrap();
        assert_eq!(original.args, before);
        start(&mut state, "work").unwrap();
        let pool = save(&state);
        let mut unknown = session(&state);
        unknown.folder = temp
            .path()
            .join("explicit other")
            .to_string_lossy()
            .into_owned();
        unknown.account_id = None;
        launch(&pool, Some(&unknown), &role(), &mut original).unwrap();
        assert_eq!(original.args, before);
        state.accounts.truncate(1);
        state.main_id = "native".into();
        let pool = save(&state);
        launch(&pool, None, &role(), &mut original).unwrap();
        assert_eq!(original.args, before);
        assert!(config_env(&pool, Default::default()).is_empty());
    }
    #[test]
    fn add_sharing_stop_and_remove_keep_login_and_user_configuration() {
        let (temp, mut state) = fixture();
        state.accounts.truncate(1);
        let core = crate::test_support::test_core_in(temp.path().join("core"));
        db::app_state_set(
            &core.db.get().unwrap(),
            super::super::KEY,
            &serde_json::to_string(&state).unwrap(),
        )
        .unwrap();
        let folder = temp.path().join("added folder");
        fs::create_dir(&folder).unwrap();
        fs::write(folder.join(".claude.json"), "own identity").unwrap();
        let added = super::super::action(
            &core,
            ClaudeAccountAction::Add {
                name: "Work".into(),
                folder: Some(folder.to_string_lossy().into_owned()),
                use_main_settings: true,
            },
        )
        .unwrap();
        let id = added.accounts.last().unwrap().id.clone();
        assert!(added.sharing[&id].using_main);
        super::super::action(&core, ClaudeAccountAction::StopSharing { id: id.clone() }).unwrap();
        assert!(!exists(&folder.join("CLAUDE.md")));
        super::super::action(&core, ClaudeAccountAction::Share { id: id.clone() }).unwrap();
        let removed = super::super::action(&core, ClaudeAccountAction::Remove { id }).unwrap();
        assert_eq!(removed.accounts.len(), 1);
        assert!(removed.sharing.is_empty());
        assert!(folder.join("skills").is_dir());
        assert!(fs::read_dir(folder.join("skills"))
            .unwrap()
            .next()
            .is_none());
        assert_eq!(
            fs::read_to_string(folder.join(".claude.json")).unwrap(),
            "own identity"
        );
    }

    #[test]
    fn changing_main_retargets_only_managed_links() {
        let (temp, mut state) = fixture();
        state.accounts.push(ClaudeAccount {
            id: "other".into(),
            name: "Other".into(),
            folder: temp
                .path()
                .join("other folder")
                .to_string_lossy()
                .into_owned(),
            native: false,
        });
        start(&mut state, "work").unwrap();
        start(&mut state, "other").unwrap();
        let previous = state.clone();
        state.main_id = "work".into();
        apply_action(
            &mut state,
            &previous,
            &ClaudeAccountAction::MakeMain { id: "work".into() },
            None,
        )
        .unwrap();
        assert!(!state.sharing.contains_key("work"));
        let work = PathBuf::from(&state.accounts[1].folder);
        let other = PathBuf::from(&state.accounts[2].folder);
        fs::write(work.join("CLAUDE.md"), "new owner memory").unwrap();
        #[cfg(unix)]
        let expected_memory = "new owner memory".to_owned();
        #[cfg(windows)]
        let expected_memory = import_text(&canonical(&work).unwrap().join("CLAUDE.md"));
        assert_eq!(
            fs::read_to_string(other.join("CLAUDE.md")).unwrap(),
            expected_memory
        );
        let config = &state.sharing["other"];
        assert_eq!(config.main_folder, canonical(&work).unwrap());
        assert!(config.links.iter().all(|link| owned(config, link).unwrap()));
        let pool = save(&state);
        assert_eq!(main_channel(&pool), Some("latest"));
    }
    #[cfg(unix)]
    #[test]
    fn failed_main_change_keeps_old_owner_and_retries_after_reload() {
        use std::os::unix::fs::PermissionsExt;
        for (locked_id, retargeting, mode) in [
            ("work", false, 0o500),
            ("zz-later", false, 0o500),
            ("zz-later", true, 0o500),
            ("work", false, 0),
            ("zz-later", false, 0),
        ] {
            let (temp, mut state) = fixture();
            write_skill(
                Path::new(&state.accounts[1].folder),
                "work-main-skill",
                "work main skill",
            );
            for id in ["other", "zz-later"] {
                state.accounts.push(ClaudeAccount {
                    id: id.into(),
                    name: id.into(),
                    folder: temp.path().join(id).to_string_lossy().into_owned(),
                    native: false,
                });
            }
            for id in ["work", "other", "zz-later"] {
                start(&mut state, id).unwrap();
            }
            if retargeting {
                for config in state.sharing.values_mut() {
                    unlink(config).unwrap();
                }
                start_for(&mut state, "other", "work").unwrap();
                state.main_change = Some(MainChange {
                    target_id: "work".into(),
                    linked_ids: state.sharing.keys().cloned().collect(),
                    retargeting: true,
                });
            }
            let mut core = crate::test_support::test_core_in(temp.path().join("core"));
            core.usage = std::sync::Arc::new(crate::usage::UsageService::without_fetch());
            let cli = temp.path().join("fake-query");
            fs::write(&cli, "#!/bin/sh\nexit 1\n").unwrap();
            fs::set_permissions(&cli, fs::Permissions::from_mode(0o700)).unwrap();
            db::set_runtime_override(&core.db, "claude-code", cli.to_str()).unwrap();
            db::app_state_set(
                &core.db.get().unwrap(),
                super::super::KEY,
                &serde_json::to_string(&state).unwrap(),
            )
            .unwrap();
            let folder = PathBuf::from(
                &state
                    .accounts
                    .iter()
                    .find(|account| account.id == locked_id)
                    .unwrap()
                    .folder,
            );
            let permissions = fs::metadata(&folder).unwrap().permissions();
            struct Restore(PathBuf, fs::Permissions);
            impl Drop for Restore {
                fn drop(&mut self) {
                    let _ = fs::set_permissions(&self.0, self.1.clone());
                }
            }
            let restore = Restore(folder.clone(), permissions);
            fs::set_permissions(&folder, fs::Permissions::from_mode(mode)).unwrap();
            let failure =
                super::super::action(&core, ClaudeAccountAction::MakeMain { id: "work".into() });
            assert!(failure.is_err(), "{locked_id}, retargeting={retargeting}");
            let persisted = super::super::load(&core.db).unwrap();
            assert_eq!(persisted.main_id, "native");
            assert_eq!(persisted.main_change.as_ref().unwrap().target_id, "work");
            assert!(persisted.sharing.contains_key("work"));
            let snapshot = super::super::snapshot(&core).unwrap();
            assert_eq!(snapshot.main_id, "native");
            assert_eq!(snapshot.pending_main_id.as_deref(), Some("work"));
            assert!(!snapshot.sharing["other"].using_main);
            assert_eq!(main_account(&core.db).unwrap().id, "native");
            let mut blocked = spec();
            assert!(launch(&core.db, Some(&session(&state)), &role(), &mut blocked).is_err());
            assert!(super::super::action(
                &core,
                ClaudeAccountAction::StopSharing { id: "other".into() }
            )
            .is_err());
            drop(restore);
            let config = &persisted.sharing[locked_id];
            if !retargeting {
                assert!(!config.links.is_empty());
                assert!(config.links.iter().all(|link| owned(config, link).unwrap()));
            }
            // A fresh service reads the persisted transition and completes the same request.
            core.claude_accounts =
                std::sync::Arc::new(super::super::AccountService::without_authentication());
            let ready =
                super::super::action(&core, ClaudeAccountAction::MakeMain { id: "work".into() })
                    .unwrap();
            assert_eq!(ready.main_id, "work");
            assert!(ready.pending_main_id.is_none());
            assert!(!ready.sharing.contains_key("work"));
            let reloaded = super::super::load(&core.db).unwrap();
            assert!(reloaded.main_change.is_none());
            let main = canonical(Path::new(&state.accounts[1].folder)).unwrap();
            for id in ["other", "zz-later"] {
                let config = &reloaded.sharing[id];
                assert_eq!(config.main_folder, main);
                assert_eq!(
                    config
                        .links
                        .iter()
                        .map(|link| link.name.as_str())
                        .collect::<std::collections::BTreeSet<_>>(),
                    [
                        "CLAUDE.md",
                        "agents",
                        "commands",
                        "settings.json",
                        "skills/work-main-skill"
                    ]
                    .into_iter()
                    .collect()
                );
                assert!(config.links.iter().all(|link| owned(config, link).unwrap()));
            }
            assert_eq!(
                fs::read_to_string(Path::new(&state.accounts[0].folder).join("CLAUDE.md")).unwrap(),
                "source memory"
            );
        }
    }
}
