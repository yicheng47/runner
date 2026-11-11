use serde::{Deserialize, Serialize};

use super::wire_enum;
use crate::protocol::{Activity, AgentStatus, Lifecycle, WaitReason};

wire_enum!(Status, Idle, {
    NeedsYou => "needsYou", Working => "working", Idle => "idle",
    Starting => "starting", Stopped => "stopped", Error => "error",
});
wire_enum!(NeedsYouReason, Unknown, {
    Approval => "approval", Answer => "answer", Unknown => "unknown",
});

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStatus {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub startup_error: Option<crate::protocol::StartupError>,
    pub status: Status,
    pub reason: Option<NeedsYouReason>,
    pub unread: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub error_acknowledged: bool,
    /// An idle mission member that lost its turn to a Runner restart: when
    /// runnerd stopped, in ms (#1187). Older hosts send none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interrupted_since: Option<i64>,
}

impl From<&AgentStatus> for SessionStatus {
    fn from(agent: &AgentStatus) -> Self {
        let (status, reason) = match agent.lifecycle {
            Lifecycle::Starting | Lifecycle::Resuming => (Status::Starting, None),
            Lifecycle::Stopped => (Status::Stopped, None),
            Lifecycle::Error => (Status::Error, None),
            Lifecycle::Running => {
                if let Some(interaction) = agent.observation.interactions.first() {
                    let reason = match interaction.reason {
                        WaitReason::Approval => NeedsYouReason::Approval,
                        WaitReason::Answer => NeedsYouReason::Answer,
                        WaitReason::Unknown => NeedsYouReason::Unknown,
                    };
                    (Status::NeedsYou, Some(reason))
                } else if agent.observation.activity == Activity::Working {
                    (Status::Working, None)
                } else {
                    (Status::Idle, None)
                }
            }
        };
        let interrupted_since = agent.interrupted_since.filter(|_| status == Status::Idle);
        Self {
            startup_error: agent.startup_error.clone(),
            status,
            reason,
            unread: agent.unread_since.is_some(),
            error_acknowledged: agent.lifecycle == Lifecycle::Error && agent.error_since.is_none(),
            interrupted_since,
        }
    }
}
