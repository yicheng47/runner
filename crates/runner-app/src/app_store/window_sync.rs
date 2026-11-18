use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use gpui::{App, AppContext as _, Context, Task};
use runner_core::protocol::node::NodeRow;
use runner_core::protocol::window::Subject;
use runner_core::protocol::{ClientResult, WindowEntry};

use super::AppStore;
use crate::surfaces::root::NativeRoot;
use crate::surfaces::windowing::checkpoint_window_layout;

#[derive(Clone, Debug)]
struct PendingWindowTabViewed {
    generation: u64,
    tab_id: String,
    member_ids: Vec<String>,
    viewed_session_id: Option<String>,
}

impl PendingWindowTabViewed {
    fn matches_subjects(&self, subjects: &[Subject], focused_session_id: Option<&str>) -> bool {
        if self.viewed_session_id.as_deref() != focused_session_id {
            return false;
        }
        if subjects.len() != self.member_ids.len() {
            return false;
        }
        subjects
            .iter()
            .zip(&self.member_ids)
            .all(|(subject, member)| matches!(subject, Subject::DirectChat(id) if id == member))
    }
}

struct WindowQueueEntry {
    closed: bool,
    cancelled: Arc<AtomicBool>,
    pending_unregister: bool,
    needs_register: bool,
    subjects: Option<(Vec<Subject>, Option<String>)>,
    focus: Option<bool>,
    focus_seq: u64,
    viewed_tab: Option<PendingWindowTabViewed>,
    tab_viewed_generation: u64,
}

impl Default for WindowQueueEntry {
    fn default() -> Self {
        Self {
            closed: false,
            cancelled: Arc::new(AtomicBool::new(false)),
            pending_unregister: false,
            needs_register: false,
            subjects: None,
            focus: None,
            focus_seq: 0,
            viewed_tab: None,
            tab_viewed_generation: 0,
        }
    }
}

#[derive(Default)]
pub(crate) struct WindowCoordinator {
    entries: HashMap<String, WindowQueueEntry>,
    next_focus_seq: u64,
    refresh_windows_pending: bool,
    pub(crate) in_flight: bool,
    task: Option<Task<()>>,
}

struct WindowBatchOp {
    label: String,
    cancelled: Arc<AtomicBool>,
    focus_seq: u64,
    needs_register: bool,
    subjects: Option<(Vec<Subject>, Option<String>)>,
    focus: Option<bool>,
    viewed_tab: Option<PendingWindowTabViewed>,
}

struct ViewedBatchOutcome {
    label: String,
    generation: u64,
    result: ClientResult<Option<NodeRow>>,
}

struct WindowBatchResult {
    errors: Vec<(String, String)>,
    viewed_outcomes: Vec<ViewedBatchOutcome>,
    window_entries: Option<Vec<WindowEntry>>,
}

impl WindowCoordinator {
    pub(crate) fn has_pending_work(&self) -> bool {
        self.refresh_windows_pending
            || self.entries.values().any(|entry| {
                entry.pending_unregister
                    || (!entry.closed
                        && (entry.needs_register
                            || entry.subjects.is_some()
                            || entry.focus.is_some()
                            || entry.viewed_tab.is_some()))
            })
    }

    pub(crate) fn should_exclude_from_snapshot(&self, label: &str) -> bool {
        self.entries
            .get(label)
            .is_some_and(|entry| entry.closed || entry.pending_unregister)
    }
}

impl AppStore {
    pub(crate) fn open_window_queue(&mut self, label: &str) {
        let entry = self
            .window_coordinator
            .entries
            .entry(label.to_owned())
            .or_default();
        if entry.closed {
            entry.closed = false;
            entry.cancelled = Arc::new(AtomicBool::new(false));
            if entry.pending_unregister {
                entry.needs_register = true;
            }
        }
    }

    pub(crate) fn enqueue_window_state(
        &mut self,
        label: &str,
        subjects: Vec<Subject>,
        focused_session_id: Option<String>,
        focus: Option<bool>,
        viewed_tab: Option<(String, Vec<String>, Option<String>)>,
        cx: &mut Context<Self>,
    ) {
        let next_focus_seq = &mut self.window_coordinator.next_focus_seq;
        let entry = self
            .window_coordinator
            .entries
            .entry(label.to_owned())
            .or_default();
        if entry.closed {
            return;
        }
        if let Some(active) = focus {
            entry.focus = Some(active);
            if active {
                *next_focus_seq = next_focus_seq.wrapping_add(1);
                entry.focus_seq = *next_focus_seq;
            } else {
                entry.tab_viewed_generation = entry.tab_viewed_generation.wrapping_add(1);
                entry.viewed_tab = None;
            }
        }
        match viewed_tab {
            Some((tab_id, member_ids, viewed_session_id)) => {
                *next_focus_seq = next_focus_seq.wrapping_add(1);
                entry.focus_seq = *next_focus_seq;
                entry.tab_viewed_generation = entry.tab_viewed_generation.wrapping_add(1);
                entry.viewed_tab = Some(PendingWindowTabViewed {
                    generation: entry.tab_viewed_generation,
                    tab_id,
                    member_ids,
                    viewed_session_id,
                });
            }
            None => {
                let compatible = entry.focus != Some(false)
                    && entry.viewed_tab.as_ref().is_some_and(|vt| {
                        vt.matches_subjects(&subjects, focused_session_id.as_deref())
                    });
                if !compatible && entry.viewed_tab.is_some() {
                    entry.tab_viewed_generation = entry.tab_viewed_generation.wrapping_add(1);
                    entry.viewed_tab = None;
                }
            }
        }
        entry.subjects = Some((subjects, focused_session_id));
        self.window_coordinator.refresh_windows_pending = true;
        if !self.window_coordinator.in_flight {
            self.start_next_window_batch(cx);
        }
    }

    pub(crate) fn enqueue_window_tab_viewed(
        &mut self,
        label: &str,
        tab_id: String,
        member_ids: Vec<String>,
        viewed_session_id: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let subjects = member_ids
            .iter()
            .cloned()
            .map(Subject::DirectChat)
            .collect();
        self.enqueue_window_state(
            label,
            subjects,
            viewed_session_id.clone(),
            Some(true),
            Some((tab_id, member_ids, viewed_session_id)),
            cx,
        );
    }

    pub(crate) fn enqueue_window_focus_event(
        &mut self,
        label: &str,
        reconnected: bool,
        cx: &mut Context<Self>,
    ) {
        if reconnected {
            if let Some(entry) = self.window_coordinator.entries.get_mut(label) {
                if !entry.closed {
                    entry.needs_register = true;
                }
            }
        }
        self.window_coordinator.refresh_windows_pending = true;
        if !self.window_coordinator.in_flight {
            self.start_next_window_batch(cx);
        }
    }

    pub(crate) fn close_window_queue(&mut self, label: &str, cx: &mut Context<Self>) {
        let entry = self
            .window_coordinator
            .entries
            .entry(label.to_owned())
            .or_default();
        entry.closed = true;
        entry.cancelled.store(true, Ordering::Release);
        entry.pending_unregister = true;
        entry.needs_register = false;
        entry.subjects = None;
        entry.focus = None;
        entry.tab_viewed_generation = entry.tab_viewed_generation.wrapping_add(1);
        entry.viewed_tab = None;
        self.window_coordinator.refresh_windows_pending = true;

        self.window_entries_epoch = self.window_entries_epoch.wrapping_add(1);
        let before = self.window_entries.len();
        self.window_entries
            .retain(|existing| existing.label != label);
        if self.window_entries.len() != before {
            self.revisions.activity = self.revisions.activity.wrapping_add(1);
            cx.notify();
            cx.defer(|cx| sync_open_windows_after_snapshot(Vec::new(), cx));
        }

        if !self.window_coordinator.in_flight {
            self.start_next_window_batch(cx);
        }
    }

    fn start_next_window_batch(&mut self, cx: &mut Context<Self>) {
        let mut unregisters = Vec::new();
        let mut ops = Vec::new();
        for (label, entry) in &mut self.window_coordinator.entries {
            if entry.pending_unregister {
                entry.pending_unregister = false;
                unregisters.push(label.clone());
            }
            if !entry.closed
                && (entry.needs_register
                    || entry.subjects.is_some()
                    || entry.focus.is_some()
                    || entry.viewed_tab.is_some())
            {
                ops.push(WindowBatchOp {
                    label: label.clone(),
                    cancelled: Arc::clone(&entry.cancelled),
                    focus_seq: entry.focus_seq,
                    needs_register: std::mem::replace(&mut entry.needs_register, false),
                    subjects: entry.subjects.take(),
                    focus: entry.focus.take(),
                    viewed_tab: entry.viewed_tab.take(),
                });
            }
        }
        let refresh_windows =
            std::mem::replace(&mut self.window_coordinator.refresh_windows_pending, false)
                || !unregisters.is_empty()
                || !ops.is_empty();
        if unregisters.is_empty() && ops.is_empty() && !refresh_windows {
            self.window_coordinator.in_flight = false;
            self.window_coordinator.task = None;
            return;
        }
        ops.sort_by(|left, right| {
            left.focus_seq
                .cmp(&right.focus_seq)
                .then_with(|| left.label.cmp(&right.label))
        });

        self.window_coordinator.in_flight = true;
        self.window_entries_epoch = self.window_entries_epoch.wrapping_add(1);
        let client = self.client.clone();
        let bg = cx.background_spawn(async move {
            let mut errors = Vec::new();
            let mut viewed_outcomes = Vec::new();
            for label in unregisters {
                let _ = client.unregister(&label);
            }
            for op in ops {
                if op.cancelled.load(Ordering::Acquire) {
                    continue;
                }
                if op.needs_register {
                    if let Err(error) = client.window_register(&op.label) {
                        errors.push((op.label.clone(), error.to_string()));
                    }
                }
                if op.cancelled.load(Ordering::Acquire) {
                    continue;
                }
                if let Some((subjects, focused)) = op.subjects {
                    if let Err(error) =
                        client.report_subjects(&op.label, subjects, focused.as_deref())
                    {
                        errors.push((op.label.clone(), error.to_string()));
                    }
                }
                if op.cancelled.load(Ordering::Acquire) {
                    continue;
                }
                if let Some(active) = op.focus {
                    if active {
                        if let Err(error) = client.mark_focused(&op.label) {
                            errors.push((op.label.clone(), error.to_string()));
                        }
                    } else {
                        let _ = client.mark_blurred(&op.label);
                    }
                }
                if op.cancelled.load(Ordering::Acquire) {
                    continue;
                }
                if let Some(viewed) = op.viewed_tab {
                    let result = client.node_mark_viewed(
                        &op.label,
                        &viewed.tab_id,
                        viewed.member_ids,
                        viewed.viewed_session_id.as_deref(),
                    );
                    viewed_outcomes.push(ViewedBatchOutcome {
                        label: op.label,
                        generation: viewed.generation,
                        result,
                    });
                }
            }
            let window_entries =
                refresh_windows.then(|| client.window_snapshot().unwrap_or_default());
            WindowBatchResult {
                errors,
                viewed_outcomes,
                window_entries,
            }
        });
        self.window_coordinator.task = Some(cx.spawn(async move |weak, cx| {
            let mut batch = bg.await;
            let _ = weak.update(cx, |this, cx| {
                let _turn = crate::stall_watchdog::enter_turn("store.window_sync_apply");
                for outcome in batch.viewed_outcomes {
                    match outcome.result {
                        Ok(Some(updated)) => {
                            let is_current = this
                                .window_coordinator
                                .entries
                                .get(&outcome.label)
                                .is_some_and(|entry| {
                                    !entry.closed
                                        && entry.tab_viewed_generation == outcome.generation
                                });
                            if is_current {
                                this.apply_node_viewed(&updated.id, updated.last_viewed_at, cx);
                            }
                        }
                        Ok(None) => {}
                        Err(error) => {
                            batch.errors.push((outcome.label, error.to_string()));
                        }
                    }
                }
                if let Some(entries) = batch.window_entries {
                    this.set_window_entries(entries, cx);
                }
                let errors = batch.errors;
                cx.defer(move |cx| sync_open_windows_after_snapshot(errors, cx));
                this.window_coordinator.in_flight = false;
                this.start_next_window_batch(cx);
            });
        }));
    }
}

fn sync_open_windows_after_snapshot(errors: Vec<(String, String)>, cx: &mut App) {
    let mut first_error_by_label = HashMap::<String, String>::new();
    for (label, error) in errors {
        first_error_by_label.entry(label).or_insert(error);
    }
    for handle in cx.windows() {
        let Some(root_handle) = handle.downcast::<NativeRoot>() else {
            continue;
        };
        let _ = root_handle.update(cx, |root, window, cx| {
            if root.closing {
                return;
            }
            if let Some(error) = first_error_by_label.remove(&root.window_label) {
                root.error = Some(error.into());
            }
            root.sync_subject_ownership(window, cx);
            cx.notify();
        });
    }
    if cx.try_global::<crate::WindowLayoutCheckpoint>().is_some() {
        checkpoint_window_layout(cx);
    }
}
