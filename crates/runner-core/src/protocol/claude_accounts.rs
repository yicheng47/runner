use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaudeAccount {
    pub id: String,
    pub name: String,
    pub folder: String,
    pub native: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaudeAuthStatus {
    pub logged_in: bool,
    pub email: Option<String>,
    pub organization: Option<String>,
    pub plan: Option<String>,
    pub signing_in: bool,
    pub sign_in_url: Option<String>,
    pub error: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ClaudeAccounts {
    pub accounts: Vec<ClaudeAccount>,
    pub main_id: String,
    pub active_id: String,
    pub statuses: std::collections::BTreeMap<String, ClaudeAuthStatus>,
    #[serde(default)]
    pub sharing: std::collections::BTreeMap<String, ClaudeAccountSharing>,
    #[serde(default)]
    pub pending_main_id: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ClaudeAccountSharing {
    pub using_main: bool,
    pub own_items: Vec<String>,
    #[serde(default)]
    pub shared_skills: usize,
    #[serde(default)]
    pub shared_plugins: usize,
}

impl ClaudeAccounts {
    pub fn multiple(&self) -> bool {
        self.accounts.len() > 1
    }
    pub fn active(&self) -> Option<&ClaudeAccount> {
        self.accounts
            .iter()
            .find(|account| account.id == self.active_id)
    }
    pub fn session_account(
        &self,
        session: Option<&ClaudeSessionAccount>,
    ) -> Option<&ClaudeAccount> {
        match session {
            Some(session) => self.accounts.iter().find(|account| {
                account.native == session.native
                    && (account.native || account.folder == session.folder)
            }),
            None => self.accounts.iter().find(|account| account.native),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaudeSessionAccount {
    pub account_id: Option<String>,
    pub folder: String,
    pub native: bool,
    #[serde(default)]
    pub pinned: bool,
    pub origin_folder: String,
    pub permission_mode: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ClaudeAccountAction {
    Add {
        name: String,
        folder: Option<String>,
        #[serde(default)]
        use_main_settings: bool,
    },
    AddAndLogin {
        name: String,
        folder: Option<String>,
        #[serde(default)]
        use_main_settings: bool,
    },
    Rename {
        id: String,
        name: String,
    },
    Remove {
        id: String,
    },
    MakeMain {
        id: String,
    },
    Use {
        id: String,
    },
    Login {
        id: String,
    },
    SignOut {
        id: String,
    },
    Cancel {
        id: String,
    },
    Share {
        id: String,
    },
    StopSharing {
        id: String,
    },
    Refresh,
}

pub fn claude_account_folder_name(name: &str) -> String {
    let mut slug = String::new();
    for character in name.trim().chars() {
        if character.is_alphanumeric() {
            slug.extend(character.to_lowercase());
        } else if !slug.ends_with('-') && !slug.is_empty() {
            slug.push('-');
        }
    }
    format!(".claude-{}", slug.trim_end_matches('-'))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sharing_counts_default_for_older_snapshots() {
        let sharing: ClaudeAccountSharing =
            serde_json::from_str(r#"{"using_main":true,"own_items":["skills"]}"#).unwrap();
        assert_eq!(sharing.shared_skills, 0);
        assert_eq!(sharing.shared_plugins, 0);
    }

    #[test]
    fn explicit_native_folder_pin_does_not_match_native_identity() {
        let native = ClaudeAccount {
            id: "native".into(),
            name: "Personal".into(),
            folder: "/home/test/.claude".into(),
            native: true,
        };
        let accounts = ClaudeAccounts {
            accounts: vec![native.clone()],
            ..Default::default()
        };
        let pin = ClaudeSessionAccount {
            account_id: None,
            folder: native.folder.clone(),
            native: false,
            pinned: true,
            origin_folder: native.folder.clone(),
            permission_mode: "default".into(),
        };
        assert!(accounts.session_account(Some(&pin)).is_none());
        assert_eq!(accounts.session_account(None), Some(&native));
        assert_eq!(
            claude_account_folder_name("  Work / Team  "),
            ".claude-work-team"
        );
        assert_eq!(claude_account_folder_name("研发"), ".claude-研发");
    }
}
