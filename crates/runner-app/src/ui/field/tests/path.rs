use super::*;

#[test]
fn a_path_compacts_to_home_then_folds_its_middle_keeping_the_last_folder() {
    let home = Some("/Users/jason");
    assert_eq!(
        compact_path_candidates("/Users/jason/repos/yicheng47", home),
        ["~/repos/yicheng47", "~/…/yicheng47"]
    );
    assert_eq!(compact_path_candidates("/Users/jason/", home), ["~"]);
    assert_eq!(
        compact_path_candidates("/Users/jasonx/repo", home),
        ["/Users/jasonx/repo", "/…/jasonx/repo", "/…/repo"],
        "a sibling of home is not under it"
    );
    assert_eq!(
        compact_path_candidates("/opt/work/projects/runner-app", None),
        [
            "/opt/work/projects/runner-app",
            "/…/work/projects/runner-app",
            "/…/projects/runner-app",
            "/…/runner-app",
        ]
    );
    assert_eq!(
        compact_path_candidates(r"C:\Users\jason\repos\runner", Some(r"C:\Users\jason")),
        [r"~\repos\runner", r"~\…\runner"]
    );
    assert_eq!(compact_path_candidates("/", home), ["/"]);
    assert_eq!(
        compact_path_candidates(r"C:\", None),
        [r"C:\"],
        "a drive root keeps its separator"
    );
    assert_eq!(compact_path_candidates(r"C:\Users", None), [r"C:\Users"]);
    assert_eq!(compact_path_candidates("dir", home), ["dir"]);
    assert_eq!(
        compact_path_candidates("very-long-parent-name/final", home),
        ["very-long-parent-name/final", "…/final"],
        "a shallow relative path folds its head to keep the last folder"
    );
    assert_eq!(
        compact_path_candidates("a/b/c", home),
        ["a/b/c", "a/…/c", "…/c"]
    );
}

#[test]
fn the_longest_fitting_display_wins_else_the_shortest() {
    let candidates = || {
        vec![
            "~/repos/yicheng47/runner".to_owned(),
            "~/…/yicheng47/runner".to_owned(),
            "~/…/runner".to_owned(),
        ]
    };
    assert_eq!(
        pick_compact_path(candidates(), |candidate| candidate.chars().count() <= 20),
        "~/…/yicheng47/runner"
    );
    assert_eq!(pick_compact_path(candidates(), |_| false), "~/…/runner");
    assert_eq!(pick_compact_path(Vec::new(), |_| true), "");
    let shallow = compact_path_candidates("very-long-parent-name/final", None);
    assert_eq!(
        pick_compact_path(shallow, |candidate| candidate.chars().count() <= 10),
        "…/final",
        "where only the folded form fits, the last folder shows"
    );
}

#[test]
fn an_unfocused_path_field_shows_what_fits_and_keeps_the_last_folder() {
    let path = "/opt/work/some-long-folder/another-long-folder/runner-app";
    let (mut visual, input) =
        open_field(200., move |focus| working_dir_text_field(focus, path, ""));
    let shown = input.read_with(&visual, |field, _| field.compact_path_shown(false));
    let shown = shown.expect("an unfocused path field compacts");
    assert_ne!(shown, path, "the full path does not fit 200 px");
    assert!(
        shown.starts_with("/…/") && shown.ends_with("/runner-app"),
        "{shown}"
    );
    let selector: &'static str = Box::leak(format!("TEXT_FIELD_COMPACT {shown}").into());
    assert!(
        visual.debug_bounds(selector).is_some(),
        "the field re-rendered with its pick, not only stored it"
    );
    assert_eq!(
        input.read_with(&visual, |field, _| field.compact_path_shown(true)),
        None,
        "focused, the field edits the full path"
    );
}

#[test]
fn working_directory_precedence_matches_main() {
    assert_eq!(
        working_dir_placeholder(Some("/runner"), "/default"),
        "/runner"
    );
    assert_eq!(working_dir_placeholder(None, "/default"), "/default");
    let home = runner_core::app_paths::home_dir()
        .expect("home directory")
        .into_os_string()
        .into_string()
        .unwrap();
    assert_eq!(working_dir_placeholder(None, ""), home);

    assert_eq!(
        effective_working_dir(" /typed ", true, "/default"),
        Some("/typed".into())
    );
    assert_eq!(effective_working_dir("", true, "/default"), None);
    assert_eq!(
        effective_working_dir("", false, "/default"),
        Some("/default".into())
    );
    assert_eq!(effective_working_dir("", false, ""), Some(home.clone()));
    assert_eq!(effective_working_dir(" \t", false, " "), Some(home));
}
