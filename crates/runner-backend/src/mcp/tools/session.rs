use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, Content};
use rmcp::{tool, tool_router, ErrorData};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::error::Error;
use crate::mcp::server::RunnerMcpHandler;
use crate::ops::session;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct StartDirectSessionArgs {
    /// Optional role ID. Omit it for a role-free runtime chat.
    #[serde(default)]
    pub role_id: Option<String>,
    /// Optional runtime registry name. With a role, this overrides the
    /// role's runtime; without a role, it starts a role-free chat.
    #[serde(default)]
    pub runtime: Option<crate::model::Runtime>,
    /// Optional model for the selected role or runtime.
    #[serde(default)]
    pub model: Option<String>,
    /// Optional reasoning effort for the selected role or runtime.
    #[serde(default)]
    pub effort: Option<String>,
    /// Optional project membership. Its cwd is used when cwd is omitted.
    #[serde(default)]
    pub project_id: Option<String>,
    /// Optional working-directory override.
    #[serde(default)]
    pub cwd: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SessionArgs {
    pub session_id: String,
}

fn command_error(error: Error) -> ErrorData {
    match error {
        Error::Msg(message) => ErrorData::invalid_request(message, None),
        other => ErrorData::internal_error(other.to_string(), None),
    }
}

fn validate_start_source(
    role_id: Option<&str>,
    runtime: Option<crate::model::Runtime>,
) -> Result<(), ErrorData> {
    if role_id.is_none() && runtime.is_none() {
        Err(ErrorData::invalid_request(
            "one of role_id or runtime is required",
            None,
        ))
    } else {
        Ok(())
    }
}

#[tool_router(router = session_router, vis = "pub(crate)")]
impl RunnerMcpHandler {
    #[tool(description = "List recent direct chats; mission sessions are excluded.")]
    pub async fn session_list(&self) -> Result<CallToolResult, ErrorData> {
        let sessions = session::session_list_recent_direct(&self.state).map_err(command_error)?;
        let statuses = self.state.sessions.status_snapshot();
        let sessions: Vec<_> = sessions
            .into_iter()
            .map(|session| {
                let activity = statuses
                    .get(&session.session_id)
                    .map(|status| status.observation.activity)
                    .unwrap_or_default();
                let mut value = serde_json::to_value(session).expect("session row serializes");
                value["activity"] = serde_json::json!(activity);
                value
            })
            .collect();
        Ok(CallToolResult::success(vec![Content::json(&sessions)?]))
    }

    #[tool(description = "Fetch one session row with its live agent status and raw activity.")]
    pub async fn session_get(
        &self,
        Parameters(args): Parameters<SessionArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let details = session::session_get_with_status(&self.state, &args.session_id)
            .map_err(command_error)?;
        Ok(CallToolResult::success(vec![Content::json(&details)?]))
    }

    #[tool(description = "Stop a direct chat or mission session and leave its row resumable.")]
    pub async fn session_stop(
        &self,
        Parameters(args): Parameters<SessionArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let state = self.state.clone();
        let session_id = args.session_id;
        let output_id = session_id.clone();
        tokio::task::spawn_blocking(move || {
            let conn = state.db.get()?;
            if crate::repo::session::get_row(&conn, &session_id)?.is_none() {
                return Err(crate::error::Error::msg(format!(
                    "session not found: {session_id}"
                )));
            }
            drop(conn);
            session::session_kill(&state, &session_id)
        })
        .await
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?
        .map_err(command_error)?;
        Ok(CallToolResult::success(vec![Content::json(
            serde_json::json!({"session_id": output_id}),
        )?]))
    }

    #[tool(description = "Stop and archive a direct chat; mission sessions are refused.")]
    pub async fn session_archive(
        &self,
        Parameters(args): Parameters<SessionArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let state = self.state.clone();
        let session_id = args.session_id;
        let output_id = session_id.clone();
        tokio::task::spawn_blocking(move || {
            let direct = match session::session_get(&state, &session_id)? {
                Some(direct) => direct,
                None => {
                    let conn = state.db.get()?;
                    return match crate::repo::session::get_row(&conn, &session_id)? {
                        Some(_) => Err(crate::error::Error::msg(format!(
                            "session {session_id} is mission-scoped; only direct chats can be archived"
                        ))),
                        None => Err(crate::error::Error::msg(format!(
                            "session not found: {session_id}"
                        ))),
                    };
                }
            };
            if direct.agent_runtime == crate::model::Runtime::Shell.key() {
                return Err(crate::error::Error::msg(format!(
                    "session {session_id} is a terminal; terminals close rather than archive"
                )));
            }
            if direct.status == crate::model::SessionStatus::Running {
                session::session_kill(&state, &session_id)?;
            }
            session::session_archive(&state, &session_id)
        })
        .await
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?
        .map_err(command_error)?;
        Ok(CallToolResult::success(vec![Content::json(
            serde_json::json!({"session_id": output_id}),
        )?]))
    }

    #[tool(description = "Resume a stopped session, continuing its conversation when available.")]
    pub async fn session_resume(
        &self,
        Parameters(args): Parameters<SessionArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let state = self.state.clone();
        let output = tokio::task::spawn_blocking(move || {
            session::session_resume(&state, &args.session_id, None, None)
        })
        .await
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?
        .map_err(command_error)?;
        Ok(CallToolResult::success(vec![Content::json(&output)?]))
    }

    #[tool(
        description = "Restart a mission slot with a fresh conversation and its cold-start brief. Reuses the session row and preserves mission messages."
    )]
    pub async fn session_restart(
        &self,
        Parameters(args): Parameters<SessionArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let state = self.state.clone();
        let output = tokio::task::spawn_blocking(move || {
            session::session_restart(&state, &args.session_id, None, None)
        })
        .await
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?
        .map_err(command_error)?;
        Ok(CallToolResult::success(vec![Content::json(&output)?]))
    }

    #[tool(
        description = "Start a direct chat for a role, an optional role runtime override, or a role-free runtime. A project's cwd is used unless cwd is explicitly provided."
    )]
    pub async fn session_start_direct(
        &self,
        Parameters(args): Parameters<StartDirectSessionArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        validate_start_source(args.role_id.as_deref(), args.runtime)?;
        let output = match (args.role_id, args.runtime) {
            (Some(role_id), runtime) => session::session_start_direct_impl(
                &self.state,
                role_id,
                runtime.map(|runtime| runtime.to_string()),
                args.model,
                args.effort,
                args.project_id,
                args.cwd,
                None,
                None,
            )
            .map_err(command_error)?,
            (None, Some(runtime)) => {
                let cwd = {
                    let conn = self
                        .state
                        .db
                        .get()
                        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
                    crate::ops::project::resolve_cwd(&conn, args.project_id.as_deref(), args.cwd)
                        .map_err(command_error)?
                };
                let session = session::session_start_runtime(
                    &self.state,
                    runtime.key(),
                    args.project_id.clone(),
                    cwd.clone(),
                    None,
                    None,
                    args.model,
                    args.effort,
                )
                .map_err(command_error)?;
                session::StartDirectSessionOutput {
                    session,
                    project_id: args.project_id,
                    cwd,
                }
            }
            (None, None) => unreachable!(),
        };
        Ok(CallToolResult::success(vec![Content::json(&output)?]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct TrackingRuntime {
        stops: std::sync::atomic::AtomicUsize,
        outputs: std::sync::Mutex<
            std::collections::HashMap<
                String,
                std::sync::mpsc::Sender<crate::session::runtime::RuntimeOutput>,
            >,
        >,
    }

    impl crate::session::runtime::SessionRuntime for TrackingRuntime {
        fn spawn(
            &self,
            spec: crate::session::runtime::SpawnSpec,
        ) -> crate::session::runtime::RuntimeResult<(
            crate::session::runtime::RuntimeSession,
            crate::session::runtime::OutputStream,
        )> {
            let (sender, receiver) = std::sync::mpsc::channel();
            self.outputs
                .lock()
                .unwrap()
                .insert(spec.session_id.clone(), sender);
            let session = crate::session::runtime::RuntimeSession {
                runtime: "tracking".into(),
                session_id: spec.session_id,
            };
            let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            Ok((
                session,
                crate::session::runtime::OutputStream::new(receiver, stop),
            ))
        }

        fn stop(
            &self,
            session: &crate::session::runtime::RuntimeSession,
        ) -> crate::session::runtime::RuntimeResult<()> {
            self.stops.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.outputs.lock().unwrap().remove(&session.session_id);
            Ok(())
        }

        fn send_bytes(
            &self,
            _session: &crate::session::runtime::RuntimeSession,
            _bytes: &[u8],
        ) -> crate::session::runtime::RuntimeResult<()> {
            Ok(())
        }

        fn send_key(
            &self,
            _session: &crate::session::runtime::RuntimeSession,
            _key: &str,
        ) -> crate::session::runtime::RuntimeResult<()> {
            Ok(())
        }

        fn resize(
            &self,
            _session: &crate::session::runtime::RuntimeSession,
            _cols: u16,
            _rows: u16,
        ) -> crate::session::runtime::RuntimeResult<()> {
            Ok(())
        }

        fn status(
            &self,
            _session: &crate::session::runtime::RuntimeSession,
        ) -> crate::session::runtime::RuntimeResult<Option<crate::session::runtime::SessionStatus>>
        {
            Ok(Some(crate::session::runtime::SessionStatus {
                alive: false,
                exit_code: Some(0),
                ..Default::default()
            }))
        }
    }

    fn result_json(result: CallToolResult) -> serde_json::Value {
        serde_json::from_str(&result.content[0].as_text().unwrap().text).unwrap()
    }

    #[test]
    fn direct_start_requires_a_role_or_runtime_and_allows_an_override() {
        assert!(validate_start_source(Some("role"), None).is_ok());
        assert!(validate_start_source(None, Some(crate::model::Runtime::Codex)).is_ok());
        assert!(validate_start_source(Some("role"), Some(crate::model::Runtime::Codex)).is_ok());
        assert!(validate_start_source(None, None).is_err());
    }

    #[tokio::test]
    async fn session_list_returns_direct_chats_only() {
        let handler = RunnerMcpHandler::new(crate::test_support::test_core());
        {
            let conn = handler.state.db.get().unwrap();
            crate::test_support::insert_test_role(&conn, "role", "coder", "codex", "codex");
            conn.execute(
                "INSERT INTO crews (id, name, created_at, updated_at)
                 VALUES ('crew', 'Crew', '2026-09-18T00:00:00Z', '2026-09-18T00:00:00Z')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO missions (id, crew_id, title, status, started_at)
                 VALUES ('mission', 'crew', 'Mission', 'running', '2026-09-18T00:00:00Z')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO sessions (id, role_id, status, started_at)
                 VALUES ('direct', 'role', 'stopped', '2026-09-18T00:00:00Z')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO sessions (id, mission_id, role_id, status, started_at)
                 VALUES ('mission-session', 'mission', 'role', 'stopped', '2026-09-18T00:00:00Z')",
                [],
            )
            .unwrap();
        }

        let rows = result_json(handler.session_list().await.unwrap());
        assert_eq!(rows.as_array().unwrap().len(), 1);
        assert_eq!(rows[0]["session_id"], "direct");
        assert_eq!(rows[0]["activity"], "unavailable");
    }

    #[tokio::test]
    async fn session_get_reports_the_managers_working_and_idle_status() {
        let handler = RunnerMcpHandler::new(crate::test_support::test_core());
        {
            let conn = handler.state.db.get().unwrap();
            crate::test_support::insert_test_role(&conn, "role", "coder", "codex", "codex");
            let mut row = crate::test_support::test_session_row(
                "direct",
                crate::model::SessionStatus::Running,
            );
            row.role_id = Some("role".into());
            crate::repo::session::insert(&conn, &row).unwrap();
        }

        assert!(handler.state.sessions.note_forwarder_transition(
            "direct",
            crate::session::manager::SessionActivityState::Busy,
            "forwarder"
        ));
        let working = result_json(
            handler
                .session_get(Parameters(SessionArgs {
                    session_id: "direct".into(),
                }))
                .await
                .unwrap(),
        );
        assert_eq!(
            working["agent_status"]["observation"]["activity"],
            "working"
        );
        assert_eq!(working["activity"], "busy");

        assert!(handler.state.sessions.note_forwarder_transition(
            "direct",
            crate::session::manager::SessionActivityState::Idle,
            "forwarder"
        ));
        let idle = result_json(
            handler
                .session_get(Parameters(SessionArgs {
                    session_id: "direct".into(),
                }))
                .await
                .unwrap(),
        );
        assert_eq!(idle["agent_status"]["observation"]["activity"], "idle");
        assert_eq!(idle["activity"], "idle");
    }

    #[tokio::test]
    async fn session_stop_preserves_a_resumable_row() {
        let handler = RunnerMcpHandler::new(crate::test_support::test_core());
        {
            let conn = handler.state.db.get().unwrap();
            crate::test_support::insert_test_role(&conn, "role", "coder", "codex", "codex");
            let mut row = crate::test_support::test_session_row(
                "direct",
                crate::model::SessionStatus::Stopped,
            );
            row.role_id = Some("role".into());
            row.agent_session_key = Some("conversation".into());
            crate::repo::session::insert(&conn, &row).unwrap();
        }
        handler
            .session_stop(Parameters(SessionArgs {
                session_id: "direct".into(),
            }))
            .await
            .unwrap();
        let row = crate::repo::session::get_row(&handler.state.db.get().unwrap(), "direct")
            .unwrap()
            .unwrap();
        assert_eq!(row.agent_session_key.as_deref(), Some("conversation"));
        assert_eq!(row.status, crate::model::SessionStatus::Stopped);

        let error = handler
            .session_stop(Parameters(SessionArgs {
                session_id: "missing".into(),
            }))
            .await
            .unwrap_err();
        assert!(error.message.contains("session not found"));
    }

    #[tokio::test]
    async fn session_archive_archives_a_chat_and_refuses_a_mission_session() {
        let handler = RunnerMcpHandler::new(crate::test_support::test_core());
        {
            let conn = handler.state.db.get().unwrap();
            crate::test_support::insert_test_role(&conn, "role", "coder", "codex", "codex");
            conn.execute(
                "INSERT INTO crews (id, name, created_at, updated_at)
                 VALUES ('crew', 'Crew', '2026-09-18T00:00:00Z', '2026-09-18T00:00:00Z')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO missions (id, crew_id, title, status, started_at)
                 VALUES ('mission', 'crew', 'Mission', 'running', '2026-09-18T00:00:00Z')",
                [],
            )
            .unwrap();
            for (id, mission_id) in [("direct", None), ("mission-session", Some("mission"))] {
                let mut row =
                    crate::test_support::test_session_row(id, crate::model::SessionStatus::Stopped);
                row.role_id = Some("role".into());
                row.mission_id = mission_id.map(str::to_owned);
                crate::repo::session::insert(&conn, &row).unwrap();
            }
        }
        handler
            .session_archive(Parameters(SessionArgs {
                session_id: "direct".into(),
            }))
            .await
            .unwrap();
        assert!(
            crate::repo::session::get_row(&handler.state.db.get().unwrap(), "direct")
                .unwrap()
                .unwrap()
                .archived_at
                .is_some()
        );
        let error = handler
            .session_archive(Parameters(SessionArgs {
                session_id: "mission-session".into(),
            }))
            .await
            .unwrap_err();
        assert!(error.message.contains("mission-scoped"));
    }

    #[tokio::test]
    async fn session_archive_refuses_a_running_terminal_without_stopping_it() {
        let mut core = crate::test_support::test_core();
        let runtime = std::sync::Arc::new(TrackingRuntime::default());
        core.sessions = crate::session::SessionManager::new(
            std::sync::Arc::clone(&core.runtime_shell_env),
            std::sync::Arc::clone(&core.runtime_discovery),
            runtime.clone(),
        );
        let temp = tempfile::tempdir().unwrap();
        let spawned = session::session_start_shell(
            &core,
            None,
            Some(temp.path().to_string_lossy().into_owned()),
            None,
            None,
        )
        .unwrap();
        let handler = RunnerMcpHandler::new(core);

        let error = handler
            .session_archive(Parameters(SessionArgs {
                session_id: spawned.id.clone(),
            }))
            .await
            .unwrap_err();

        assert!(error
            .message
            .contains("terminals close rather than archive"));
        assert_eq!(runtime.stops.load(std::sync::atomic::Ordering::SeqCst), 0);
        let row = crate::repo::session::get_row(&handler.state.db.get().unwrap(), &spawned.id)
            .unwrap()
            .unwrap();
        assert_eq!(row.status, crate::model::SessionStatus::Running);
        assert!(row.archived_at.is_none());
        assert_eq!(
            handler.state.sessions.agent_status(&spawned.id).lifecycle,
            crate::session::status::Lifecycle::Running
        );

        session::session_kill(&handler.state, &spawned.id).unwrap();
    }
}
