// Error types shared between the GPUI binary and the `runner` CLI.
//
// Narrower than the backend error in `runner-backend/src/error.rs` — no
// rusqlite dependency leaks in here. Backend code wraps this via
// `From<runner_core::Error>`.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("json: {0}")]
    Json(#[from] serde_json::Error),

    #[error("{0}")]
    Msg(String),
}

impl Error {
    pub fn msg(s: impl Into<String>) -> Self {
        Error::Msg(s.into())
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
