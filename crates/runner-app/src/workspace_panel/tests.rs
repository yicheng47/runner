use super::*;
use gpui::prelude::*;

#[test]
fn workspace_roots_use_session_then_project_then_default_without_blank_values() {
    assert_eq!(
        workspace_root(Some(" /session "), Some("/project"), "/default"),
        Some(PathBuf::from("/session"))
    );
    assert_eq!(
        workspace_root(Some(" "), Some("/project"), "/default"),
        Some(PathBuf::from("/project"))
    );
    assert_eq!(
        workspace_root(None, None, "/default"),
        Some(PathBuf::from("/default"))
    );
    assert_eq!(workspace_root(None, Some(""), " "), None);
}

#[test]
fn cached_tree_expands_only_requested_directories() {
    let mut cx = gpui::TestAppContext::single();
    let panel = cx.new(|cx| WorkspacePanel::new(&[PanelBuiltin::Info], cx));
    panel.update(&mut cx, |panel, _| {
        let root = PathBuf::from("/workspace");
        let folder = root.join("folder");
        let entry = |path: PathBuf, is_dir| DirectoryEntry {
            name: path.file_name().unwrap().to_string_lossy().into_owned(),
            path,
            is_dir,
            is_symlink: false,
            ignored: false,
        };
        panel.root = Some(root.clone());
        panel.directories.insert(
            root.clone(),
            DirectoryState {
                loaded: true,
                entries: vec![
                    entry(folder.clone(), true),
                    entry(root.join("notes.md"), false),
                ],
                ..Default::default()
            },
        );
        panel.directories.insert(
            folder.clone(),
            DirectoryState {
                loaded: true,
                entries: vec![entry(folder.join("child.txt"), false)],
                ..Default::default()
            },
        );
        assert_eq!(panel.rows().len(), 2);
        panel.expanded.insert(folder);
        let rows = panel.rows();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[1].depth, 1);
        assert_eq!(rows[1].entry.name, "child.txt");
        assert_eq!(rows[2].entry.name, "notes.md");
    });
}

#[test]
fn changing_root_discards_file_tabs_and_inflight_generation() {
    let mut cx = gpui::TestAppContext::single();
    let panel = cx.new(|cx| WorkspacePanel::new(&[PanelBuiltin::Info], cx));
    panel.update(&mut cx, |panel, cx| {
        panel.root = Some(PathBuf::from("/old"));
        panel
            .tabs
            .open(TabKind::File(PathBuf::from("/old/private.txt")));
        let generation = panel.generation;
        panel.set_root(None, cx);
        assert!(panel.generation > generation);
        assert_eq!(panel.active_builtin(), Some(PanelBuiltin::Info));
        assert!(panel.previews.is_empty());
        assert!(panel.directories.is_empty());
    });
}

struct PanelHarness {
    panel: gpui::Entity<WorkspacePanel>,
    width: f32,
    _observe: gpui::Subscription,
}

impl gpui::Render for PanelHarness {
    fn render(
        &mut self,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        use gpui::{div, px};
        window.set_rem_size(px(16.));
        let header = self
            .panel
            .update(cx, |panel, cx| panel.render_header(self.width, window, cx));
        let body = if self.panel.read(cx).active_builtin().is_some() {
            div()
                .id("panel-builtin")
                .debug_selector(|| "PANEL_BUILTIN".into())
                .child("Info")
                .into_any_element()
        } else {
            self.panel.clone().into_any_element()
        };
        div()
            .w(px(self.width))
            .h(px(650.))
            .flex()
            .flex_col()
            .child(header)
            .child(div().min_h(px(0.)).flex_1().child(body))
    }
}

#[test]
fn files_open_deduplicate_refresh_and_close_without_losing_builtin_views() {
    use gpui::VisualTestContext;
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join("nested")).unwrap();
    let path = temp.path().join("README.md");
    std::fs::write(&path, "# File preview\n\nRead only.").unwrap();
    let mut cx = gpui::TestAppContext::single();
    let panel = cx.new(|cx| WorkspacePanel::new(&[PanelBuiltin::Info], cx));
    panel.update(&mut cx, |panel, cx| {
        panel.set_root(Some(temp.path().to_path_buf()), cx)
    });
    let host = cx.add_window(|_, cx| {
        let observe = cx.observe(&panel, |_, _, cx| cx.notify());
        PanelHarness {
            panel: panel.clone(),
            width: 320.,
            _observe: observe,
        }
    });
    let mut visual = VisualTestContext::from_window(host.into(), &cx);
    visual.run_until_parked();
    assert!(visual.debug_bounds("PANEL_BUILTIN").is_some());
    panel.update(&mut visual, |panel, cx| panel.show_files(cx));
    visual.run_until_parked();
    panel.read_with(&visual, |panel, _| {
        assert_eq!(panel.rows().len(), 2);
        assert!(panel.rows()[0].entry.is_dir);
    });
    panel.update(&mut visual, |panel, cx| panel.open_file(path.clone(), cx));
    visual.run_until_parked();
    let file_id = panel.read_with(&visual, |panel, _| {
        let id = panel.tabs.active;
        assert!(matches!(panel.tabs.selected().kind, TabKind::File(_)));
        assert!(panel.previews[&id].view.is_some());
        assert!(panel.previews[&id].error.is_none());
        id
    });
    panel.update(&mut visual, |panel, cx| {
        panel.open_file(temp.path().join("nested/../README.md"), cx)
    });
    visual.run_until_parked();
    panel.read_with(&visual, |panel, _| {
        assert_eq!(panel.tabs.active, file_id);
        assert_eq!(panel.tabs.tabs.len(), 3);
    });
    std::fs::write(&path, "# Updated\n\nVisible after refresh.").unwrap();
    panel.update(&mut visual, |panel, cx| panel.refresh(true, cx));
    visual.run_until_parked();
    panel.read_with(&visual, |panel, _| {
        assert!(
            matches!(&panel.previews[&file_id].snapshot.as_ref().unwrap().content,
            workspace_files::PreviewContent::Markdown(text) if text.contains("Updated"))
        );
    });
    std::fs::remove_file(&path).unwrap();
    panel.update(&mut visual, |panel, cx| panel.refresh(true, cx));
    visual.run_until_parked();
    panel.read_with(&visual, |panel, _| {
        assert!(panel.previews[&file_id].error.is_some())
    });
    panel.update(&mut visual, |panel, cx| panel.close(file_id, cx));
    visual.run_until_parked();
    panel.read_with(&visual, |panel, _| {
        assert_eq!(panel.tabs.selected().kind, TabKind::Files)
    });
    panel.update(&mut visual, |panel, cx| {
        panel.activate_builtin(PanelBuiltin::Info, cx)
    });
    visual.run_until_parked();
    assert!(visual.debug_bounds("PANEL_BUILTIN").is_some());
    host.update(&mut visual, |_, window, _| window.remove_window())
        .unwrap();
}

#[test]
fn tabs_overflow_within_narrow_and_wide_panel_headers() {
    use gpui::{px, VisualTestContext};
    let mut cx = gpui::TestAppContext::single();
    let panel = cx.new(|cx| WorkspacePanel::new(&[PanelBuiltin::Roles, PanelBuiltin::Meta], cx));
    panel.update(&mut cx, |panel, _| {
        for index in 0..12 {
            panel.tabs.open(TabKind::File(PathBuf::from(format!(
                "/workspace/long-filename-{index}.md"
            ))));
        }
    });
    let host = cx.add_window(|_, cx| {
        let observe = cx.observe(&panel, |_, _, cx| cx.notify());
        PanelHarness {
            panel: panel.clone(),
            width: 240.,
            _observe: observe,
        }
    });
    let mut visual = VisualTestContext::from_window(host.into(), &cx);
    for width in [240., 480., 720.] {
        host.update(&mut visual, |host, _, cx| {
            host.width = width;
            cx.notify();
        })
        .unwrap();
        visual.run_until_parked();
        let strip = visual.debug_bounds("WORKSPACE_PANEL_TABS").unwrap();
        let active = visual.debug_bounds("WORKSPACE_PANEL_TAB 15").unwrap();
        assert!(active.left() >= strip.left() - px(1.));
        assert!(
            active.right() <= strip.right() + px(1.),
            "width={width} active={active:?} strip={strip:?}"
        );
        assert!(active.size.height > px(0.));
    }
    host.update(&mut visual, |_, window, _| window.remove_window())
        .unwrap();
}

#[test]
fn pending_open_cannot_steal_focus_or_repopulate_a_changed_workspace() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("notes.md");
    std::fs::write(&path, "# Notes").unwrap();
    let mut cx = gpui::TestAppContext::single();
    let panel = cx.new(|cx| WorkspacePanel::new(&[PanelBuiltin::Info], cx));
    panel.update(&mut cx, |panel, cx| {
        panel.set_root(Some(temp.path().to_path_buf()), cx);
        panel.open_file(path.clone(), cx);
        let new_id = panel.tabs.open(TabKind::New);
        panel.select(new_id, cx);
    });
    cx.run_until_parked();
    panel.read_with(&cx, |panel, _| {
        assert_eq!(panel.tabs.selected().kind, TabKind::New);
        assert!(panel.opening.is_empty());
        assert!(panel.previews.is_empty());
    });
    panel.update(&mut cx, |panel, cx| {
        panel.open_file(path.clone(), cx);
        panel.set_root(None, cx);
    });
    cx.run_until_parked();
    panel.read_with(&cx, |panel, _| {
        assert!(panel.root.is_none());
        assert_eq!(panel.active_builtin(), Some(PanelBuiltin::Info));
        assert!(panel.previews.is_empty());
    });
}

#[test]
fn collapsed_ancestors_keep_cached_children_but_stop_polling_them() {
    let mut cx = gpui::TestAppContext::single();
    let panel = cx.new(|cx| WorkspacePanel::new(&[PanelBuiltin::Info], cx));
    panel.update(&mut cx, |panel, _| {
        let root = PathBuf::from("/workspace");
        let parent = root.join("parent");
        let child = parent.join("child");
        let folder = |path: PathBuf| DirectoryEntry {
            name: path.file_name().unwrap().to_string_lossy().into_owned(),
            path,
            is_dir: true,
            is_symlink: false,
            ignored: false,
        };
        panel.root = Some(root.clone());
        panel.directories.insert(
            root.clone(),
            DirectoryState {
                loaded: true,
                entries: vec![folder(parent.clone())],
                ..Default::default()
            },
        );
        panel.directories.insert(
            parent.clone(),
            DirectoryState {
                loaded: true,
                entries: vec![folder(child.clone())],
                ..Default::default()
            },
        );
        panel.expanded.insert(parent.clone());
        panel.expanded.insert(child.clone());
        assert_eq!(panel.visible_directories().len(), 3);
        panel.expanded.remove(&parent);
        assert_eq!(panel.visible_directories(), [root]);
        assert!(panel.expanded.contains(&child));
    });
}

#[test]
fn panel_width_clamps_saved_widths_and_drag_widths_to_available_host_space() {
    assert_eq!(bounded_panel_width(960., 800.), 560.);
    assert_eq!(bounded_panel_width(320., 1200.), 320.);
    assert_eq!(bounded_panel_width(960., 400.), 160.);
    assert_eq!(bounded_panel_width(960., 200.), 0.);
    assert_eq!(bounded_panel_width(960., 1200. / 1.5 - 200. - 16.), 344.);
}
