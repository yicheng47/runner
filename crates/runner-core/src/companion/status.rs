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
    pub status: Status,
    pub reason: Option<NeedsYouReason>,
    pub unread: bool,
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
        Self {
            status,
            reason,
            unread: agent.unread_since.is_some(),
        }
    }
}
