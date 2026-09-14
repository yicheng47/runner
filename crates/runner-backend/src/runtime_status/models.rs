use std::collections::HashSet;
use std::io::{Read, Seek};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime};

use serde::{Deserialize, Serialize};

use super::{direct_chat_path, effective_runtime_command, SharedDiscoveryState, SharedShellEnv};
use crate::db::DbPool;
use crate::events::EventChannel;
use crate::model::Runtime;
use crate::ops::runtime::RuntimeCatalogOption;
use crate::session::process::{prepare_headless_fork, ProcessTree};
use crate::shell_path::LoginShellEnv;

const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_CATALOG_BYTES: u64 = 32 * 1024 * 1024;
const CACHE_KEY: &str = "codex_model_catalog_v1";
const CACHE_TTL_SECONDS: i64 = 10 * 60;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct CatalogSource {
    command: String,
    codex_cache: Option<PathBuf>,
    executable_modified: Option<SystemTime>,
}

#[derive(Serialize, Deserialize)]
struct CachedCatalog {
    source: CatalogSource,
    captured_at: i64,
    models: Vec<RuntimeCatalogOption>,
}

impl CachedCatalog {
    fn is_fresh(&self, now: i64) -> bool {
        (0..CACHE_TTL_SECONDS).contains(&now.saturating_sub(self.captured_at))
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ModelDiscovery {
    revision: u64,
    codex: Option<(String, Vec<RuntimeCatalogOption>)>,
}

impl ModelDiscovery {
    pub(crate) fn for_command(&self, command: Option<&str>) -> Option<&[RuntimeCatalogOption]> {
        self.codex
            .as_ref()
            .filter(|(source, _)| Some(source.as_str()) == command)
            .map(|(_, models)| models.as_slice())
    }

    fn finish(&mut self, revision: u64, command: String, models: Vec<RuntimeCatalogOption>) {
        if self.revision == revision && !models.is_empty() {
            self.codex = Some((command, models));
        }
    }
}

pub(crate) fn refresh(
    pool: &DbPool,
    shell_env: &SharedShellEnv,
    discovery: &SharedDiscoveryState,
    force: bool,
    events: &EventChannel,
) {
    let revision = {
        let Ok(mut state) = discovery.write() else {
            return;
        };
        state.models.revision += 1;
        state.models.revision
    };
    let Ok(command) = effective_runtime_command(Runtime::Codex, pool, shell_env, discovery) else {
        return;
    };
    let Ok(env) = shell_env.read().map(|env| env.clone()) else {
        return;
    };
    let source = CatalogSource {
        executable_modified: std::fs::metadata(&command.command)
            .and_then(|metadata| metadata.modified())
            .ok(),
        command: command.command,
        codex_cache: codex_cache_path(),
    };
    let cached = pool.get().ok().and_then(|conn| {
        crate::db::app_state_get(&conn, CACHE_KEY)
            .ok()
            .flatten()
            .and_then(|value| serde_json::from_str::<CachedCatalog>(&value).ok())
            .filter(|cache| cache.source == source && !cache.models.is_empty())
    });
    if let Some(cached) = &cached {
        if let Ok(mut state) = discovery.write() {
            state
                .models
                .finish(revision, source.command.clone(), cached.models.clone());
        }
        events.emit("runtime/changed", &());
        if !force && cached.is_fresh(chrono::Utc::now().timestamp()) {
            return;
        }
    }
    let models = discover(
        &source.command,
        &env,
        source.codex_cache.as_deref(),
        PROBE_TIMEOUT,
    );
    if models.is_empty() {
        return;
    }
    let cached = CachedCatalog {
        source,
        captured_at: chrono::Utc::now().timestamp(),
        models,
    };
    if let Ok(mut state) = discovery.write() {
        if state.models.revision != revision {
            return;
        }
        state.models.finish(
            revision,
            cached.source.command.clone(),
            cached.models.clone(),
        );
        let persisted = (|| -> crate::error::Result<()> {
            let conn = pool.get()?;
            crate::db::app_state_set(&conn, CACHE_KEY, &serde_json::to_string(&cached)?)
        })();
        if let Err(error) = persisted {
            log::warn!("Codex model cache could not be saved: {error}");
        }
    }
    events.emit("runtime/changed", &());
}

fn codex_cache_path() -> Option<PathBuf> {
    std::env::var_os("CODEX_HOME")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(|| runner_core::app_paths::home_dir().map(|home| home.join(".codex")))
        .map(|home| home.join("models_cache.json"))
}

fn discover(
    executable: &str,
    env: &LoginShellEnv,
    cache: Option<&Path>,
    timeout: Duration,
) -> Vec<RuntimeCatalogOption> {
    let models = probe(executable, env, timeout).unwrap_or_default();
    if !models.is_empty() {
        return models;
    }
    cache
        .and_then(|path| std::fs::File::open(path).ok())
        .and_then(|file| read_catalog(file).ok())
        .unwrap_or_default()
}

fn probe(
    executable: &str,
    env: &LoginShellEnv,
    timeout: Duration,
) -> std::io::Result<Vec<RuntimeCatalogOption>> {
    let mut output = tempfile::tempfile()?;
    let mut command = Command::new(executable);
    command
        .args(["debug", "models"])
        .envs(&env.vars)
        .env("PATH", direct_chat_path(env))
        .stdin(Stdio::null())
        .stdout(output.try_clone()?)
        .stderr(Stdio::null());
    prepare_headless_fork(&mut command);
    let mut child = command.spawn()?;
    let tree = match ProcessTree::adopt(child.id()) {
        Ok(tree) => tree,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
    };
    let started = Instant::now();
    let result = loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break Ok(()),
            Ok(Some(_)) => break Err(std::io::Error::other("Codex model query failed")),
            Err(error) => break Err(error),
            Ok(None) if started.elapsed() >= timeout => {
                break Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "Codex model query timed out",
                ));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
        }
    };
    let _ = tree.terminate();
    let _ = child.wait();
    result?;
    output.rewind()?;
    read_catalog(output)
}

fn read_catalog(input: impl Read) -> std::io::Result<Vec<RuntimeCatalogOption>> {
    let mut bytes = Vec::new();
    input.take(MAX_CATALOG_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_CATALOG_BYTES {
        return Ok(Vec::new());
    }
    Ok(parse_catalog(&bytes))
}

fn parse_catalog(bytes: &[u8]) -> Vec<RuntimeCatalogOption> {
    #[derive(Deserialize)]
    struct Catalog {
        models: Vec<Model>,
    }
    #[derive(Deserialize)]
    struct Model {
        slug: String,
        #[serde(default)]
        display_name: String,
        #[serde(default)]
        description: String,
        visibility: String,
    }
    let Ok(catalog) = serde_json::from_slice::<Catalog>(bytes) else {
        return Vec::new();
    };
    let mut seen = HashSet::new();
    catalog
        .models
        .into_iter()
        .filter(|model| model.visibility == "list")
        .filter_map(|model| {
            let value = model.slug.trim().to_owned();
            if value.is_empty() || !seen.insert(value.clone()) {
                return None;
            }
            let label = model.display_name.trim();
            let label = if label.is_empty() { &value } else { label }.to_owned();
            let description = model.description.trim();
            Some(RuntimeCatalogOption {
                value,
                label,
                description: (!description.is_empty()).then(|| description.to_owned()),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const CATALOG: &str = r#"{"models":[
        {"slug":"gpt-6-astra","display_name":"GPT-6-Astra","description":"Capable coding model","visibility":"list"},
        {"slug":"internal","visibility":"hide"},
        {"slug":"gpt-6-astra","visibility":"list"},
        {"slug":" future-model ","visibility":"list"},
        {"slug":" ","visibility":"list"}
    ]}"#;

    #[test]
    fn catalog_keeps_visible_models_and_upstream_order_without_duplicates() {
        let models = parse_catalog(CATALOG.as_bytes());
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].value, "gpt-6-astra");
        assert_eq!(models[0].label, "GPT-6-Astra");
        assert_eq!(
            models[0].description.as_deref(),
            Some("Capable coding model")
        );
        assert_eq!(models[1].value, "future-model");
        assert_eq!(models[1].label, "future-model");
        assert_eq!(models[1].description, None);
        for invalid in ["not JSON", "{}", r#"{"models":[]}"#] {
            assert!(parse_catalog(invalid.as_bytes()).is_empty());
        }
    }

    #[test]
    fn stale_results_and_failed_refreshes_preserve_last_successful_catalog() {
        let mut state = ModelDiscovery {
            revision: 2,
            ..Default::default()
        };
        state.finish(2, "codex".into(), parse_catalog(CATALOG.as_bytes()));
        state.finish(1, "old-codex".into(), parse_catalog(CATALOG.as_bytes()));
        state.finish(2, "codex".into(), Vec::new());
        assert_eq!(state.for_command(Some("codex")).unwrap().len(), 2);
        assert!(state.for_command(Some("different-codex")).is_none());
        assert!(state.for_command(None).is_none());
    }

    #[test]
    fn cache_expires_after_ten_minutes_and_rejects_future_timestamps() {
        let cache = CachedCatalog {
            source: CatalogSource {
                command: "codex".into(),
                codex_cache: None,
                executable_modified: None,
            },
            captured_at: 1_000,
            models: parse_catalog(CATALOG.as_bytes()),
        };
        assert!(cache.is_fresh(1_000));
        assert!(cache.is_fresh(1_000 + CACHE_TTL_SECONDS - 1));
        assert!(!cache.is_fresh(1_000 + CACHE_TTL_SECONDS));
        assert!(!cache.is_fresh(999));
    }

    #[cfg(unix)]
    fn executable(dir: &Path, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("codex with spaces");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[cfg(unix)]
    #[test]
    fn probe_uses_cli_args_and_falls_back_to_owned_cache() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("models_cache.json");
        std::fs::write(&cache, CATALOG).unwrap();
        let executable = executable(
            dir.path(),
            &format!(
                "test \"$1 $2\" = 'debug models' || exit 1\nprintf '%s' '{}'",
                CATALOG
            ),
        );
        let env = LoginShellEnv::default();
        assert_eq!(
            probe(executable.to_str().unwrap(), &env, PROBE_TIMEOUT)
                .unwrap()
                .len(),
            2
        );
        std::fs::write(&executable, "#!/bin/sh\nexit 1\n").unwrap();
        assert_eq!(
            discover(
                executable.to_str().unwrap(),
                &env,
                Some(&cache),
                PROBE_TIMEOUT
            )
            .len(),
            2
        );
        assert!(discover(executable.to_str().unwrap(), &env, None, PROBE_TIMEOUT).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn probe_timeout_terminates_its_process() {
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("pid");
        let executable = executable(
            dir.path(),
            &format!("echo $$ > '{}'\nexec sleep 30", pid_file.display()),
        );
        let error = probe(
            executable.to_str().unwrap(),
            &LoginShellEnv::default(),
            Duration::from_secs(1),
        )
        .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        let pid: i32 = std::fs::read_to_string(pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
    }

    #[cfg(unix)]
    #[test]
    fn refresh_reads_the_override_and_updates_the_shared_runtime_catalog() {
        let dir = tempfile::tempdir().unwrap();
        let executable = executable(dir.path(), &format!("printf '%s' '{}'", CATALOG));
        let core = crate::test_support::test_core();
        crate::db::set_runtime_override(&core.db, "codex", executable.to_str()).unwrap();
        refresh(
            &core.db,
            &core.runtime_shell_env,
            &core.runtime_discovery,
            false,
            &core.events,
        );
        let catalog = crate::ops::runtime::runtime_catalog(&core).unwrap();
        assert_eq!(
            catalog[0]
                .models
                .iter()
                .map(|model| model.value.as_str())
                .collect::<Vec<_>>(),
            ["", "gpt-6-astra", "future-model"]
        );
        assert_eq!(catalog[0].models[1].label, "GPT-6-Astra");
        assert!(catalog[1].models.iter().any(|model| model.value == "opus"));
    }

    #[cfg(unix)]
    #[test]
    fn persistent_cache_skips_relaunch_probe_until_expired_or_forced() {
        use crate::shell_path::DiscoveryState;
        use std::sync::{Arc, RwLock};

        let dir = tempfile::tempdir().unwrap();
        let executable = executable(
            dir.path(),
            &format!("printf . >> \"$0.calls\"\nprintf '%s' '{}'", CATALOG),
        );
        let calls = PathBuf::from(format!("{}.calls", executable.display()));
        let db_path = dir.path().join("runner.sqlite");
        let env = Arc::new(RwLock::new(LoginShellEnv::default()));
        let events = EventChannel::new();
        {
            let pool = crate::db::open_pool(&db_path).unwrap();
            crate::db::set_runtime_override(&pool, "codex", executable.to_str()).unwrap();
            let discovery = Arc::new(RwLock::new(DiscoveryState::startup(None, None)));
            refresh(&pool, &env, &discovery, false, &events);
            assert_eq!(std::fs::read_to_string(&calls).unwrap(), ".");
        }
        let pool = crate::db::open_pool(&db_path).unwrap();
        let discovery = Arc::new(RwLock::new(DiscoveryState::startup(None, None)));
        refresh(&pool, &env, &discovery, false, &events);
        assert_eq!(std::fs::read_to_string(&calls).unwrap(), ".");
        assert_eq!(
            discovery
                .read()
                .unwrap()
                .models
                .for_command(executable.to_str())
                .unwrap()
                .len(),
            2
        );

        refresh(&pool, &env, &discovery, true, &events);
        assert_eq!(std::fs::read_to_string(&calls).unwrap(), "..");
        {
            let conn = pool.get().unwrap();
            let value = crate::db::app_state_get(&conn, CACHE_KEY).unwrap().unwrap();
            let mut cache: CachedCatalog = serde_json::from_str(&value).unwrap();
            cache.captured_at -= CACHE_TTL_SECONDS;
            crate::db::app_state_set(&conn, CACHE_KEY, &serde_json::to_string(&cache).unwrap())
                .unwrap();
        }
        refresh(&pool, &env, &discovery, false, &events);
        assert_eq!(std::fs::read_to_string(&calls).unwrap(), "...");

        std::fs::write(
            &executable,
            format!(
                "#!/bin/sh\nprintf . >> \"$0.calls\"\nprintf '%s' '{}'\n# updated CLI",
                CATALOG
            ),
        )
        .unwrap();
        refresh(&pool, &env, &discovery, false, &events);
        assert_eq!(std::fs::read_to_string(&calls).unwrap(), "....");
    }
}
