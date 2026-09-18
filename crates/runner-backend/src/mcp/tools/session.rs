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
        Ok(CallToolResult::success(vec![Content::json(&sessions)?]))
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
    }
}
