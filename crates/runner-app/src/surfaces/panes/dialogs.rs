use super::*;

impl NativeRoot {
    pub(crate) fn render_terminal_close_confirm(&self, cx: &mut Context<Self>) -> AnyElement {
        let confirm = self
            .terminal_close_confirm
            .as_ref()
            .expect("archive confirm is open");
        let (title, body, confirm_label, pending_label, icon) = match &confirm.target {
            TerminalCloseTarget::ArchiveAll {
                confirmation_body, ..
            } => (
                crate::i18n::text("ui.archive_tab"),
                confirmation_body.resolve().to_string(),
                crate::i18n::text("ui.archive"),
                crate::i18n::text("ui.archiving"),
                "trash.svg",
            ),
            TerminalCloseTarget::ArchiveChatPane { session_id, .. } => {
                let entry = self.front_page_session(session_id, cx);
                let label = entry
                    .as_ref()
                    .map(|entry| entry.label(None))
                    .unwrap_or_else(|| crate::i18n::text("ui.this_chat").into());
                let running = entry
                    .as_ref()
                    .is_some_and(|entry| entry.status == SessionStatus::Running);
                (
                    crate::i18n::text("ui.archive_chat"),
                    archive_chat_confirm_body(&label, running),
                    crate::i18n::text("ui.archive_chat_action"),
                    crate::i18n::text("ui.archiving"),
                    "archive.svg",
                )
            }
        };
        let root = cx.entity();
        let confirm_root = root.clone();
        ConfirmDialog::new(
            title,
            body,
            confirm_label,
            pending_label,
            false,
            Rc::new(move |window, cx| {
                confirm_root.update(cx, |this, cx| this.confirm_terminal_close(window, cx));
            }),
            Rc::new(move |window, cx| {
                root.update(cx, |this, cx| this.cancel_terminal_close(window, cx));
            }),
        )
        .focus_handle(confirm.focus.clone())
        .icon(icon)
        .into_any_element()
    }

    pub(crate) fn render_fork_confirm(&self, cx: &mut Context<Self>) -> AnyElement {
        let confirm = self.fork_confirm.as_ref().expect("fork confirm is open");
        let label = self
            .front_page_session(&confirm.session_id, cx)
            .map(|entry| entry.label(None))
            .unwrap_or_else(|| crate::i18n::text("ui.this_chat").into());
        let body = crate::i18n::tf(
            "format.start_a_new_chat_from_with_its_full_conversation_history_the_fork_opens_in_a_n",
            &[("label", (label).to_string())],
        );
        let busy = confirm.pending;
        let root = cx.entity();
        let confirm_root = root.clone();
        ConfirmDialog::new(
            crate::i18n::text("ui.fork_chat"),
            body,
            crate::i18n::text("ui.fork"),
            crate::i18n::text("ui.forking"),
            busy,
            Rc::new(move |window, cx| {
                confirm_root.update(cx, |this, cx| this.confirm_fork_chat(window, cx));
            }),
            Rc::new(move |_, cx| {
                root.update(cx, |this, cx| this.cancel_fork_chat(cx));
            }),
        )
        .icon("git-fork.svg")
        .variant(ButtonVariant::Primary)
        .into_any_element()
    }
}
