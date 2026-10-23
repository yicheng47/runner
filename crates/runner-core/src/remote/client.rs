use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Computer {
    pub id: String,
    pub name: String,
    pub info: super::HostInfo,
    pub fingerprint: String,
    pub online: bool,
    pub connected_at: Option<String>,
    pub last_seen_at: Option<String>,
    #[serde(rename = "self")]
    pub is_self: bool,
    pub paired: bool,
}

#[derive(Default, Deserialize)]
#[serde(default)]
pub struct Computers {
    pub hosts: Vec<Computer>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RemoteTerminal {
    pub computer: String,
    pub session: String,
    pub cols: u16,
    pub rows: u16,
    pub take_control: bool,
}

impl RemoteTerminal {
    pub fn address(&self) -> String {
        terminal_address(&self.computer, &self.session)
    }
}
pub fn terminal_address(computer: &str, session: &str) -> String {
    format!("remote:{computer}:{session}")
}
pub fn terminal_target(address: &str) -> Option<(&str, &str)> {
    address.strip_prefix("remote:")?.split_once(':')
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decodes_account_host_proto_json_and_preserves_terminal_address() {
        let response: Computers = serde_json::from_str(r#"{"hosts":[{"id":"18446744073709551615","name":"Studio","info":{},"fingerprint":"abcd","online":true,"connectedAt":"2026-10-23T00:00:00Z","lastSeenAt":"2026-10-22T00:00:00Z","self":true}]}"#).unwrap();
        let computer = &response.hosts[0];
        assert!(computer.is_self);
        assert!(computer.online);
        assert!(!computer.paired);
        assert_eq!(computer.id, "18446744073709551615");
        assert!(computer.connected_at.is_some());
        assert!(computer.last_seen_at.is_some());
        let config = RemoteTerminal {
            computer: "42".into(),
            session: "session".into(),
            cols: 80,
            rows: 24,
            take_control: true,
        };
        assert_eq!(terminal_target(&config.address()), Some(("42", "session")));
        assert_eq!(terminal_target("local-session"), None);
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RemoteOverview(pub crate::companion::Overview);
impl std::fmt::Debug for RemoteOverview {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RemoteOverview([redacted])")
    }
}

#[cfg(test)]
#[test]
fn client_debug_keeps_snapshot_output_and_overview_bodies_private() {
    use crate::protocol::terminal::{TerminalController, TerminalFrame, TerminalSnapshot};
    let frame = TerminalFrame::Output {
        seq: 1,
        bytes: b"terminal-canary".to_vec(),
    };
    let snapshot = TerminalSnapshot {
        seq: 1,
        cols: 80,
        rows: 24,
        controller: TerminalController::Desktop,
        bytes: b"terminal-canary".to_vec(),
        unfinished_len: 0,
        preceding_char: Some('x'),
    };
    assert_eq!(format!("{frame:?}"), "TerminalFrame([redacted])");
    assert_eq!(format!("{snapshot:?}"), "TerminalSnapshot([redacted])");
    let overview: RemoteOverview = serde_json::from_str(
        r#"{"projects":[{"id":"1","name":"body-canary"}],"tabs":[],"missions":[],"sessions":{}}"#,
    )
    .unwrap();
    assert_eq!(format!("{overview:?}"), "RemoteOverview([redacted])");
    assert!(serde_json::to_string(&overview)
        .unwrap()
        .contains("body-canary"));
}
