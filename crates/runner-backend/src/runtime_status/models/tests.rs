use super::*;

fn catalog(model: &str) -> ModelCatalog {
    ModelCatalog {
        models: vec![option(model.into(), model.into(), None)],
        default_model: None,
    }
}

#[test]
fn cache_ttl_coalescing_and_failure_cooldown() {
    let source = source(Runtime::Codex, "/missing/codex");
    let mut state = ModelDiscovery::default();
    assert!(state.begin(Runtime::Codex, &source, false, 100));
    assert!(!state.begin(Runtime::Codex, &source, true, 100));
    state.finish(
        Runtime::Codex,
        source.clone(),
        100,
        Some(catalog("last-good")),
    );
    assert!(!state.begin(Runtime::Codex, &source, false, 699));
    assert!(state.begin(Runtime::Codex, &source, false, 700));
    assert_eq!(
        state.catalog(Runtime::Codex, &source),
        Some(&catalog("last-good"))
    );
    state.finish(Runtime::Codex, source.clone(), 700, None);
    assert_eq!(
        state.runtimes[&Runtime::Codex]
            .cached
            .as_ref()
            .unwrap()
            .captured_at,
        100
    );
    assert!(!state.begin(Runtime::Codex, &source, false, 701));
    assert!(state.begin(Runtime::Codex, &source, true, 701));
    state.finish(Runtime::Codex, source.clone(), 701, None);
    assert!(state.begin(Runtime::Codex, &source, false, 1301));
    assert!(state
        .catalog(
            Runtime::Codex,
            &super::source(Runtime::Codex, "/another/codex")
        )
        .is_none());
}

#[test]
fn existing_offline_cache_survives_the_smaller_schema() {
    let pool = crate::db::open_in_memory().unwrap();
    let source = source(Runtime::Codex, "/missing/codex");
    let record = CatalogRecord {
        version: CACHE_VERSION,
        source: source.clone(),
        captured_at: 100,
        catalog: catalog("cached"),
    };
    let mut legacy = serde_json::to_value([&record]).unwrap();
    legacy[0]["provenance"] = "query".into();
    legacy[0]["source"]["selectors"] = serde_json::json!({"AWS_PROFILE":"old-profile"});
    legacy[0]["catalog"]["models"][0]["efforts"] = serde_json::json!([]);
    crate::db::app_state_set(
        &pool.get().unwrap(),
        &cache_key(Runtime::Codex),
        &legacy.to_string(),
    )
    .unwrap();
    let cached = read_cached(&pool, Runtime::Codex).unwrap();
    assert_eq!(cached.source, source);
    assert_eq!(cached.catalog, record.catalog);
    legacy[0]["version"] = 999.into();
    crate::db::app_state_set(
        &pool.get().unwrap(),
        &cache_key(Runtime::Codex),
        &legacy.to_string(),
    )
    .unwrap();
    assert!(read_cached(&pool, Runtime::Codex).is_none());
}

#[cfg(unix)]
mod process_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use std::sync::RwLock;

    struct Fixture {
        dir: tempfile::TempDir,
        command: PathBuf,
        pool: Arc<DbPool>,
        discovery: SharedDiscoveryState,
        env: SharedShellEnv,
        runtime: Runtime,
    }

    impl Fixture {
        fn new(runtime: Runtime, body: &str) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let command = script(dir.path(), "agent with spaces", body);
            let pool = Arc::new(crate::db::open_pool(&dir.path().join("runner.db")).unwrap());
            crate::db::set_runtime_override(&pool, runtime.key(), command.to_str()).unwrap();
            Self {
                dir,
                command,
                pool,
                discovery: state(),
                env: Arc::new(RwLock::new(LoginShellEnv::default())),
                runtime,
            }
        }

        fn refresh(&self, force: bool) {
            request(
                &self.pool,
                &self.env,
                &self.discovery,
                &EventChannel::new(),
                &[self.runtime],
                force,
            );
            wait_until(|| {
                !self
                    .discovery
                    .read()
                    .unwrap()
                    .models
                    .runtimes
                    .get(&self.runtime)
                    .is_some_and(|state| state.in_flight)
            });
        }

        fn catalog(&self) -> Option<ModelCatalog> {
            let source = current_source(self.runtime, &self.pool, &self.env, &self.discovery)?;
            self.discovery
                .read()
                .unwrap()
                .models
                .catalog(self.runtime, &source)
                .cloned()
        }
    }

    fn state() -> SharedDiscoveryState {
        Arc::new(RwLock::new(crate::shell_path::DiscoveryState::startup(
            None, None,
        )))
    }

    fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn wait_until(condition: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !condition() {
            assert!(Instant::now() < deadline, "query did not finish");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn restart_reuses_cache_and_failed_refreshes_keep_last_good() {
        let body = r#"cd "$(dirname "$0")"
printf . >> calls
case "$(cat mode 2>/dev/null)" in
  fail) exit 3;;
  empty) exit 0;;
  malformed) printf '{';;
  crash) kill -9 $$;;
  *) test "$1 $2" = 'debug models' || exit 1
     printf '%s' '{"models":[{"slug":"cached-model","visibility":"list","supported_reasoning_levels":[{"effort":"high"}]}]}' ;;
esac"#;
        let mut fixture = Fixture::new(Runtime::Codex, body);
        let mut expected = catalog("cached-model");
        expected.models[0].supported_efforts = Some(vec!["high".into()]);
        fixture.refresh(false);
        assert_eq!(fixture.catalog(), Some(expected.clone()));
        let captured_at = read_cached(&fixture.pool, Runtime::Codex)
            .unwrap()
            .captured_at;
        fixture.discovery = state();
        fixture.pool =
            Arc::new(crate::db::open_pool(&fixture.dir.path().join("runner.db")).unwrap());
        fixture.refresh(false);
        assert_eq!(fixture.catalog(), Some(expected.clone()));
        assert_eq!(
            std::fs::read_to_string(fixture.dir.path().join("calls")).unwrap(),
            "."
        );
        for failure in ["fail", "empty", "malformed", "crash"] {
            std::fs::write(fixture.dir.path().join("mode"), failure).unwrap();
            fixture.refresh(true);
            assert_eq!(fixture.catalog(), Some(expected.clone()));
            assert_eq!(
                read_cached(&fixture.pool, Runtime::Codex).unwrap().catalog,
                expected
            );
            assert_eq!(
                read_cached(&fixture.pool, Runtime::Codex)
                    .unwrap()
                    .captured_at,
                captured_at
            );
        }
        let calls = std::fs::read_to_string(fixture.dir.path().join("calls")).unwrap();
        fixture.refresh(false);
        assert_eq!(
            std::fs::read_to_string(fixture.dir.path().join("calls")).unwrap(),
            calls
        );
        std::fs::write(fixture.dir.path().join("mode"), "ok").unwrap();
        fixture.refresh(true);
        assert_eq!(fixture.catalog(), Some(expected));
    }

    #[test]
    fn claude_uses_control_only_query_and_unsupported_runtimes_are_skipped() {
        let body = format!("test \"$*\" = '-p --input-format stream-json --output-format stream-json --include-partial-messages --verbose --safe-mode --no-session-persistence' || exit 1\nread request\nprintf '%s' \"$request\" > \"$(dirname \"$0\")/request\"\nprintf '%s' '{}'", claude::STREAM);
        let fixture = Fixture::new(Runtime::ClaudeCode, &body);
        fixture.refresh(false);
        assert_eq!(fixture.catalog().unwrap().models.len(), 3);
        let request: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(fixture.dir.path().join("request")).unwrap(),
        )
        .unwrap();
        assert_eq!(request["request"]["subtype"], "list_models");
        let fixture = Fixture::new(Runtime::Trae, "exit 1");
        fixture.refresh(true);
        assert!(!fixture
            .discovery
            .read()
            .unwrap()
            .models
            .runtimes
            .contains_key(&Runtime::Trae));
    }

    #[test]
    fn concurrent_refreshes_share_one_query_and_obsolete_results_are_discarded() {
        let fixture = Fixture::new(
            Runtime::Codex,
            "sleep 0.2\nprintf '%s' '{\"models\":[{\"slug\":\"old\",\"visibility\":\"list\"}]}'",
        );
        request(
            &fixture.pool,
            &fixture.env,
            &fixture.discovery,
            &EventChannel::new(),
            &[Runtime::Codex],
            false,
        );
        assert!(!fixture.discovery.write().unwrap().models.begin(
            Runtime::Codex,
            &source(Runtime::Codex, fixture.command.to_str().unwrap()),
            true,
            now()
        ));
        let replacement = script(
            fixture.dir.path(),
            "replacement",
            "printf '%s' '{\"models\":[{\"slug\":\"new\",\"visibility\":\"list\"}]}'",
        );
        crate::db::set_runtime_override(&fixture.pool, Runtime::Codex.key(), replacement.to_str())
            .unwrap();
        wait_until(|| fixture.catalog() == Some(catalog("new")));
        assert_eq!(
            read_cached(&fixture.pool, Runtime::Codex).unwrap().catalog,
            catalog("new")
        );
    }

    #[test]
    fn spawn_failure_timeout_and_cache_write_failure_are_contained() {
        let fixture = Fixture::new(
            Runtime::Codex,
            "echo $$ > \"$(dirname \"$0\")/pid\"\nexec sleep 30",
        );
        let result = run(Query {
            executable: fixture.command.to_str().unwrap(),
            args: &[],
            stdin: None,
            env: &LoginShellEnv::default(),
            timeout: Duration::from_secs(2),
        });
        assert_eq!(result, Err(Reason::Timeout));
        let pid: i32 = std::fs::read_to_string(fixture.dir.path().join("pid"))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
        assert_eq!(
            run(Query {
                executable: "/definitely/missing/agent",
                args: &[],
                stdin: None,
                env: &LoginShellEnv::default(),
                timeout: Duration::from_millis(100)
            }),
            Err(Reason::QueryFailed)
        );
        let fixture = Fixture::new(
            Runtime::Codex,
            "printf '%s' '{\"models\":[{\"slug\":\"memory\",\"visibility\":\"list\"}]}'",
        );
        fixture.pool.get().unwrap().execute_batch("CREATE TRIGGER fail_cache BEFORE INSERT ON _app_state WHEN NEW.key LIKE 'runtime_model_catalog:%' BEGIN SELECT RAISE(ABORT, 'disk full'); END;").unwrap();
        fixture.refresh(false);
        assert_eq!(fixture.catalog(), Some(catalog("memory")));
        assert!(read_cached(&fixture.pool, Runtime::Codex).is_none());
    }
}
