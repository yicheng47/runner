use std::collections::HashMap;
use std::io::{Read, Seek, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::{
    direct_chat_path, effective_runtime_command, RuntimeCommandSource, SharedDiscoveryState,
    SharedShellEnv,
};
use crate::db::DbPool;
use crate::events::EventChannel;
use crate::model::Runtime;
use crate::ops::runtime::RuntimeCatalogOption;
use crate::session::process::{prepare_headless_fork, ProcessTree};
use crate::shell_path::LoginShellEnv;

mod claude;
mod codex;
mod pi;

pub(crate) const DISCOVERY_RUNTIMES: [Runtime; 3] =
    [Runtime::Codex, Runtime::ClaudeCode, Runtime::Pi];
const REFRESH_SECONDS: i64 = 10 * 60;
const MAX_OUTPUT_BYTES: u64 = 32 * 1024 * 1024;
const CACHE_VERSION: u32 = 4;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ModelSource {
    command: String,
    executable: Option<Fingerprint>,
    config_home: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Fingerprint {
    modified_ms: Option<i64>,
    len: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ModelCatalog {
    pub(crate) models: Vec<RuntimeCatalogOption>,
    pub(crate) default_model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CatalogRecord {
    version: u32,
    source: ModelSource,
    captured_at: i64,
    catalog: ModelCatalog,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reason {
    Timeout,
    QueryFailed,
    InvalidOutput,
    EmptyCatalog,
}

impl Reason {
    fn as_str(self) -> &'static str {
        match self {
            Self::Timeout => "timeout",
            Self::QueryFailed => "query_failed",
            Self::InvalidOutput => "invalid_output",
            Self::EmptyCatalog => "empty_catalog",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ModelDiscovery {
    runtimes: HashMap<Runtime, RuntimeModels>,
}

#[derive(Debug, Clone, Default)]
struct RuntimeModels {
    loaded: bool,
    cached: Option<CatalogRecord>,
    in_flight: bool,
    last_attempt: Option<(ModelSource, i64)>,
}

impl ModelDiscovery {
    pub(crate) fn catalog(&self, runtime: Runtime, source: &ModelSource) -> Option<&ModelCatalog> {
        self.runtimes
            .get(&runtime)?
            .cached
            .as_ref()
            .filter(|record| &record.source == source)
            .map(|record| &record.catalog)
    }

    fn begin(&mut self, runtime: Runtime, source: &ModelSource, force: bool, now: i64) -> bool {
        let state = self.runtimes.entry(runtime).or_default();
        if state.in_flight {
            return false;
        }
        let recent = |at| (0..REFRESH_SECONDS).contains(&now.saturating_sub(at));
        if !force
            && (state
                .cached
                .as_ref()
                .is_some_and(|record| &record.source == source && recent(record.captured_at))
                || state
                    .last_attempt
                    .as_ref()
                    .is_some_and(|(last, at)| last == source && recent(*at)))
        {
            return false;
        }
        state.in_flight = true;
        true
    }

    fn finish(
        &mut self,
        runtime: Runtime,
        source: ModelSource,
        now: i64,
        catalog: Option<ModelCatalog>,
    ) -> Option<CatalogRecord> {
        let state = self.runtimes.entry(runtime).or_default();
        state.in_flight = false;
        state.last_attempt = Some((source.clone(), now));
        let record = CatalogRecord {
            version: CACHE_VERSION,
            source,
            captured_at: now,
            catalog: catalog?,
        };
        state.cached = Some(record.clone());
        Some(record)
    }
}

pub(crate) fn load_cached(pool: &DbPool, discovery: &SharedDiscoveryState, events: &EventChannel) {
    let mut changed = false;
    for runtime in DISCOVERY_RUNTIMES {
        if discovery.read().is_ok_and(|state| {
            state
                .models
                .runtimes
                .get(&runtime)
                .is_some_and(|state| state.loaded)
        }) {
            continue;
        }
        let cached = read_cached(pool, runtime);
        if let Ok(mut state) = discovery.write() {
            let state = state.models.runtimes.entry(runtime).or_default();
            if !state.loaded {
                state.loaded = true;
                changed |= cached.is_some();
                state.cached = cached;
            }
        }
    }
    if changed {
        events.emit("runtime/changed", &());
    }
}

pub(crate) fn request(
    pool: &Arc<DbPool>,
    shell_env: &SharedShellEnv,
    discovery: &SharedDiscoveryState,
    events: &EventChannel,
    runtimes: &[Runtime],
    force: bool,
) {
    load_cached(pool, discovery, events);
    for &runtime in runtimes
        .iter()
        .filter(|runtime| DISCOVERY_RUNTIMES.contains(runtime))
    {
        let Some(source) = current_source(runtime, pool, shell_env, discovery) else {
            continue;
        };
        if !discovery
            .write()
            .is_ok_and(|mut state| state.models.begin(runtime, &source, force, now()))
        {
            continue;
        }
        let pool = Arc::clone(pool);
        let shell_env = Arc::clone(shell_env);
        let discovery = Arc::clone(discovery);
        let events = events.clone();
        std::thread::spawn(move || {
            let env = shell_env.read().map(|env| env.clone()).unwrap_or_default();
            let started = Instant::now();
            let (method, result) = match runtime {
                Runtime::Codex => ("debug models", codex::query(&source.command, &env)),
                Runtime::ClaudeCode => ("list_models", claude::query(&source.command, &env)),
                Runtime::Pi => ("--offline --list-models", pi::query(&source.command, &env)),
                _ => unreachable!(),
            };
            let duration_ms = started.elapsed().as_millis();
            let current = current_source(runtime, &pool, &shell_env, &discovery);
            let applied = current.as_ref() == Some(&source);
            let catalog = match result {
                Ok(catalog) => {
                    log::info!("runtime_model_query_succeeded runtime={runtime} method={method} models={} duration_ms={duration_ms} applied={applied}", catalog.models.len());
                    applied.then_some(catalog)
                }
                Err(reason) => {
                    log::debug!("runtime_model_query_failed runtime={runtime} reason={} duration_ms={duration_ms}", reason.as_str());
                    None
                }
            };
            let captured_at = now();
            if let Some(catalog) = &catalog {
                // Queries stay coalesced during the write, without blocking catalog readers.
                persist(
                    &pool,
                    runtime,
                    &CatalogRecord {
                        version: CACHE_VERSION,
                        source: source.clone(),
                        captured_at,
                        catalog: catalog.clone(),
                    },
                );
            }
            let changed = discovery.write().is_ok_and(|mut state| {
                state
                    .models
                    .finish(runtime, source, captured_at, catalog)
                    .is_some()
            });
            if changed {
                events.emit("runtime/changed", &());
            }
            if !applied && current.is_some() {
                request(&pool, &shell_env, &discovery, &events, &[runtime], false);
            }
        });
    }
}

fn current_source(
    runtime: Runtime,
    pool: &DbPool,
    shell_env: &SharedShellEnv,
    discovery: &SharedDiscoveryState,
) -> Option<ModelSource> {
    let command = effective_runtime_command(runtime, pool, shell_env, discovery).ok()?;
    (command.source != RuntimeCommandSource::Catalog).then(|| source(runtime, &command.command))
}

pub(crate) fn source(runtime: Runtime, command: &str) -> ModelSource {
    let executable = std::fs::metadata(command).ok().map(|metadata| Fingerprint {
        modified_ms: metadata
            .modified()
            .ok()
            .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
            .and_then(|elapsed| i64::try_from(elapsed.as_millis()).ok()),
        len: metadata.len(),
    });
    let config_home = match runtime {
        Runtime::Codex => std::env::var_os("CODEX_HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .or_else(|| runner_core::app_paths::home_dir().map(|home| home.join(".codex"))),
        Runtime::ClaudeCode => std::env::var_os("CLAUDE_CONFIG_DIR")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .or_else(|| runner_core::app_paths::home_dir().map(|home| home.join(".claude"))),
        Runtime::Pi => runner_core::app_paths::home_dir().map(|home| home.join(".pi/agent")),
        _ => {
            return ModelSource {
                command: command.into(),
                executable,
                config_home: None,
            }
        }
    };
    ModelSource {
        command: command.into(),
        executable,
        config_home,
    }
}

fn cache_key(runtime: Runtime) -> String {
    format!("runtime_model_catalog:{}", runtime.key())
}

fn read_cached(pool: &DbPool, runtime: Runtime) -> Option<CatalogRecord> {
    let conn = pool.get().ok()?;
    let json = crate::db::app_state_get(&conn, &cache_key(runtime)).ok()??;
    // Keep reading the existing record envelope so this simplification preserves the offline cache.
    serde_json::from_str::<Vec<CatalogRecord>>(&json)
        .ok()?
        .into_iter()
        .find(|record| record.version == CACHE_VERSION && !record.catalog.models.is_empty())
}

fn persist(pool: &DbPool, runtime: Runtime, record: &CatalogRecord) {
    let written = (|| -> crate::error::Result<()> {
        let conn = pool.get()?;
        crate::db::app_state_set(
            &conn,
            &cache_key(runtime),
            &serde_json::to_string(&[record])?,
        )
    })();
    if let Err(error) = written {
        log::debug!("runtime_model_cache_write_failed runtime={runtime} error={error}");
    }
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

struct Query<'a> {
    executable: &'a str,
    args: &'a [&'a str],
    stdin: Option<&'a [u8]>,
    env: &'a LoginShellEnv,
    timeout: Duration,
}

/// Runs one bounded, headless query off the UI thread. Owned processes and
/// their descendants are terminated on timeout and reaped on every path.
fn run(query: Query<'_>) -> Result<Vec<u8>, Reason> {
    match run_process(query) {
        Ok(output) => Ok(output),
        Err(error) if error.kind() == std::io::ErrorKind::TimedOut => Err(Reason::Timeout),
        Err(error) if error.kind() == std::io::ErrorKind::InvalidData => Err(Reason::InvalidOutput),
        Err(_) => Err(Reason::QueryFailed),
    }
}

fn run_process(query: Query<'_>) -> std::io::Result<Vec<u8>> {
    let mut output = tempfile::tempfile()?;
    let mut command = Command::new(query.executable);
    command
        .args(query.args)
        .envs(&query.env.vars)
        .env("PATH", direct_chat_path(query.env))
        .stdin(if query.stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(output.try_clone()?)
        .stderr(Stdio::null());
    if let Some(home) = runner_core::app_paths::home_dir() {
        command.current_dir(home);
    }
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
    if let Some(bytes) = query.stdin {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(bytes);
        }
    }
    let started = Instant::now();
    let result = loop {
        if output
            .metadata()
            .is_ok_and(|metadata| metadata.len() > MAX_OUTPUT_BYTES)
        {
            break Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "model query output exceeded limit",
            ));
        }
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break Ok(()),
            Ok(Some(_)) => break Err(std::io::Error::other("model query failed")),
            Err(error) => break Err(error),
            Ok(None) if started.elapsed() >= query.timeout => {
                break Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "model query timed out",
                ));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
        }
    };
    let _ = tree.terminate();
    let _ = child.wait();
    result?;
    output.rewind()?;
    read_bounded(output, MAX_OUTPUT_BYTES)
}

fn read_bounded(input: impl Read, limit: u64) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    input.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        bytes.clear();
    }
    Ok(bytes)
}

fn option(value: String, label: String, description: Option<String>) -> RuntimeCatalogOption {
    RuntimeCatalogOption {
        value,
        label,
        description,
        supported_efforts: None,
    }
}

fn trimmed(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

#[cfg(test)]
mod tests;
