use runner_app::ui::SelectOption;
use runner_core::protocol::ClaudeAccounts;

pub(super) fn options(accounts: &ClaudeAccounts) -> Vec<SelectOption> {
    let main = accounts
        .accounts
        .iter()
        .find(|a| a.id == accounts.main_id)
        .map(|a| a.name.as_str())
        .unwrap_or_default();
    accounts
        .accounts
        .iter()
        .map(|account| {
            let mut tags = Vec::new();
            if account.id == accounts.main_id {
                tags.push(crate::i18n::message("accounts.main"));
            }
            if account.id == accounts.active_id {
                tags.push(crate::i18n::message("accounts.active"));
            }
            let sharing = accounts
                .sharing
                .get(&account.id)
                .is_some_and(|s| s.using_main);
            let settings = if sharing {
                crate::i18n::tf("integration.uses_settings", &[("main", main.into())])
            } else {
                crate::i18n::text("integration.own_settings").into()
            };
            let identity = if accounts
                .statuses
                .get(&account.id)
                .is_some_and(|status| status.logged_in)
            {
                account.folder.as_str()
            } else {
                crate::i18n::text("accounts.not_signed_in")
            };
            SelectOption::new(account.id.clone(), account.name.clone())
                .tags(tags)
                .description(format!("{settings} · {identity}"))
        })
        .collect()
}

pub(super) fn shared_note(
    accounts: &ClaudeAccounts,
    id: Option<&str>,
    mcp: bool,
) -> Option<(String, String, String)> {
    let account = accounts
        .accounts
        .iter()
        .find(|a| Some(a.id.as_str()) == id)?;
    if !accounts
        .sharing
        .get(&account.id)
        .is_some_and(|s| s.using_main)
    {
        return None;
    }
    let main = accounts
        .accounts
        .iter()
        .find(|a| a.id == accounts.main_id)?;
    Some((
        crate::i18n::tf(
            if mcp {
                "integration.shared_mcp"
            } else {
                "integration.shared_skills"
            },
            &[
                ("account", account.name.clone()),
                ("main", main.name.clone()),
            ],
        ),
        crate::i18n::tf("integration.show_main", &[("main", main.name.clone())]),
        main.id.clone(),
    ))
}

#[test]
fn account_options_show_names_tags_and_signed_out_identity() {
    let accounts = runner_core::protocol::ClaudeAccounts {
        accounts: vec![runner_core::protocol::ClaudeAccount {
            id: "main".into(),
            name: "Personal".into(),
            folder: "/tmp/personal".into(),
            native: false,
        }],
        main_id: "main".into(),
        active_id: "main".into(),
        ..Default::default()
    };
    let signed_out = options(&accounts);
    assert_eq!(signed_out[0].label.as_ref(), "Personal");
    assert!(signed_out[0]
        .description
        .as_ref()
        .unwrap()
        .contains(crate::i18n::text("accounts.not_signed_in")));
    let mut accounts = accounts;
    accounts.statuses.insert(
        "main".into(),
        runner_core::protocol::ClaudeAuthStatus {
            logged_in: true,
            ..Default::default()
        },
    );
    assert!(options(&accounts)[0]
        .description
        .as_ref()
        .unwrap()
        .contains("/tmp/personal"));
}
