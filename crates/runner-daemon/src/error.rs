use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoleRejection {
    NotRunning,
    SpawnPending,
    HandleInUse,
    HandleRemoved,
    FirstTurnTooLarge,
    NotInMission,
    AlreadyRemoved,
    NoSession,
    Resuming,
}

impl RoleRejection {
    pub(crate) const ALL: [Self; 9] = [
        Self::NotRunning,
        Self::SpawnPending,
        Self::HandleInUse,
        Self::HandleRemoved,
        Self::FirstTurnTooLarge,
        Self::NotInMission,
        Self::AlreadyRemoved,
        Self::NoSession,
        Self::Resuming,
    ];

    pub(crate) const fn companion_message(self) -> &'static str {
        match self {
            Self::NotRunning => "The mission is not running.",
            Self::SpawnPending => {
                "A session in this mission is still starting; try again once it is up."
            }
            Self::HandleInUse => "That handle is already in use in this mission.",
            Self::HandleRemoved => {
                "That handle was already removed from this mission; choose another handle."
            }
            Self::FirstTurnTooLarge => "The mission brief is too large to launch that role.",
            Self::NotInMission => "That role is not in this mission.",
            Self::AlreadyRemoved => "That role was already removed from this mission.",
            Self::NoSession => "That role has no session in this mission.",
            Self::Resuming => "That role is being resumed or restarted; try again once it is up.",
        }
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("sqlite pool: {0}")]
    Pool(#[from] r2d2::Error),

    #[error("json: {0}")]
    Json(#[from] serde_json::Error),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("direct input timed out after {timeout_ms} ms waiting for session {session_id}")]
    DirectInputTimeout { session_id: String, timeout_ms: u64 },

    #[error(transparent)]
    RuntimeVersionGate(#[from] runner_core::protocol::runtime::RuntimeVersionGate),

    #[error("{message}")]
    RoleRejected {
        kind: RoleRejection,
        message: String,
    },

    #[error("{0}")]
    Msg(String),
}

impl Error {
    pub fn role_rejected(kind: RoleRejection, message: impl Into<String>) -> Self {
        Self::RoleRejected {
            kind,
            message: message.into(),
        }
    }

    pub fn context(self, context: &str) -> Self {
        match self {
            Self::RuntimeVersionGate(_) => self,
            error => Self::msg(format!("{context}: {error}")),
        }
    }

    pub fn into_client_error(self) -> runner_core::protocol::ClientError {
        let message = self.to_string();
        let version_gate = match self {
            Self::RuntimeVersionGate(gate) => Some(gate),
            _ => None,
        };
        runner_core::protocol::ClientError {
            message,
            version_gate,
        }
    }

    pub fn msg(s: impl Into<String>) -> Self {
        Error::Msg(s.into())
    }
}

// Lift shared-core errors into the app error so command code can `?` across
// the boundary. We flatten to existing variants rather than adding a `Core`
// wrapper that'd need its own Display.
impl From<runner_core::Error> for Error {
    fn from(e: runner_core::Error) -> Self {
        match e {
            runner_core::Error::Io(err) => Error::Io(err),
            runner_core::Error::Json(err) => Error::Json(err),
            runner_core::Error::Msg(s) => Error::Msg(s),
        }
    }
}

impl serde::Serialize for Error {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

impl From<runner_core::protocol::ClientError> for Error {
    fn from(error: runner_core::protocol::ClientError) -> Self {
        match error.version_gate {
            Some(gate) => Self::RuntimeVersionGate(gate),
            None => Self::msg(error.message),
        }
    }
}
