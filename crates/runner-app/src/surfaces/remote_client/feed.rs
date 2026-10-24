use super::model::ClientState;
use crate::surfaces::front_page::{FrontPageClient, FrontPageTarget};
use futures::{
    channel::{mpsc, oneshot},
    StreamExt,
};
use gpui::{AppContext, Context, Entity, WeakEntity};
use runner_core::companion::{FeedAppended, FeedEntry, FeedPage};
use std::collections::HashSet;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex, Weak,
};

pub(super) struct SharedMissionFeed {
    feed: WeakEntity<MissionFeed>,
    follows: Weak<FeedFollow>,
}

#[derive(Default)]
struct FeedFollow {
    owner: AtomicU64,
    requests: Mutex<()>,
}

pub(crate) struct MissionFeed {
    pub(crate) entries: Vec<FeedEntry>,
    pub(crate) before: Option<String>,
    pub(crate) error: Option<String>,
    pub(crate) loading: bool,
    pub(crate) older_loading: bool,
    client: FrontPageClient,
    mission: String,
    generation: u64,
    appends: Vec<FeedEntry>,
    release: Option<oneshot::Sender<Option<oneshot::Receiver<()>>>>,
    follow_done: Option<oneshot::Receiver<()>>,
    follows: Arc<FeedFollow>,
    follow_owner: u64,
    refresh_again: bool,
    history_loaded: bool,
    retry_older: bool,
    catch_up_ids: Option<HashSet<String>>,
    _events: Vec<gpui::Task<()>>,
}

impl ClientState {
    pub(crate) fn mission_feed(
        &mut self,
        client: FrontPageClient,
        mission: String,
        cx: &mut Context<Self>,
    ) -> Entity<MissionFeed> {
        let FrontPageTarget::Remote {
            computer,
            fingerprint,
            ..
        } = &client.target
        else {
            unreachable!()
        };
        self.mission_feeds
            .retain(|_, entry| entry.feed.upgrade().is_some() || entry.follows.upgrade().is_some());
        let key = (computer.clone(), fingerprint.clone(), mission.clone());
        if let Some(feed) = self
            .mission_feeds
            .get(&key)
            .and_then(|entry| entry.feed.upgrade())
        {
            feed.update(cx, |feed, cx| feed.set_client(client, cx));
            return feed;
        }
        let follows = self
            .mission_feeds
            .get(&key)
            .and_then(|entry| entry.follows.upgrade())
            .unwrap_or_default();
        let daemon = self.client.clone();
        let feed = cx.new(|cx| MissionFeed::new(client, daemon, mission, follows.clone(), cx));
        self.mission_feeds.insert(
            key,
            SharedMissionFeed {
                feed: feed.downgrade(),
                follows: Arc::downgrade(&follows),
            },
        );
        feed
    }
}

impl MissionFeed {
    fn new(
        client: FrontPageClient,
        daemon: runner_core::protocol::DaemonClient,
        mission: String,
        follows: Arc<FeedFollow>,
        cx: &mut Context<Self>,
    ) -> Self {
        let (tx, mut rx) = mpsc::unbounded();
        let mut events = daemon.subscribe();
        let receive = cx.background_spawn(async move {
            loop {
                match events.recv().await {
                    Ok(event) if event.name == "remote/feed" => {
                        if tx.unbounded_send(event.payload).is_err() {
                            break;
                        }
                    }
                    Ok(_) => {}
                    Err(runner_core::protocol::EventError::Lagged(_)) => {
                        if tx.unbounded_send(serde_json::Value::Null).is_err() {
                            break;
                        }
                    }
                    Err(runner_core::protocol::EventError::Closed) => break,
                }
            }
        });
        let apply = cx.spawn(async move |weak, cx| {
            while let Some(payload) = rx.next().await {
                if weak
                    .update(cx, |feed: &mut Self, cx| {
                        if !feed.client.valid() {
                            return;
                        }
                        if payload.is_null() {
                            feed.refresh(cx);
                            return;
                        }
                        let FrontPageTarget::Remote { computer, .. } = &feed.client.target else {
                            return;
                        };
                        if payload.get("computer").and_then(serde_json::Value::as_str)
                            != Some(computer)
                        {
                            return;
                        }
                        let Some(value) = payload.get("feed") else {
                            return;
                        };
                        let Ok(batch) = serde_json::from_value::<FeedAppended>(value.clone())
                        else {
                            return;
                        };
                        if batch.mission != feed.mission {
                            return;
                        }
                        if feed.loading {
                            feed.appends.extend(batch.entries);
                        } else {
                            merge_append(&mut feed.entries, batch.entries);
                        }
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        let follow_owner = follows.owner.fetch_add(1, Ordering::SeqCst) + 1;
        let mut feed = Self {
            entries: Vec::new(),
            before: None,
            error: None,
            loading: false,
            older_loading: false,
            client,
            mission,
            generation: 0,
            appends: Vec::new(),
            release: None,
            follow_done: None,
            follows,
            follow_owner,
            refresh_again: false,
            history_loaded: false,
            retry_older: false,
            catch_up_ids: None,
            _events: vec![receive, apply],
        };
        feed.refresh(cx);
        feed
    }

    fn release_current(&mut self) {
        if let Some(release) = self.release.take() {
            let _ = release.send(self.follow_done.take());
        }
    }

    pub(crate) fn set_client(&mut self, client: FrontPageClient, cx: &mut Context<Self>) {
        if self.client.target != client.target {
            self.release_current();
            self.follow_owner = self.follows.owner.fetch_add(1, Ordering::SeqCst) + 1;
            self.client = client;
            self.generation += 1;
            self.loading = false;
            self.older_loading = false;
            self.appends.clear();
            self.refresh(cx);
        }
    }

    pub(crate) fn refresh(&mut self, cx: &mut Context<Self>) {
        if !self.client.supports(&["follow", "unfollow", "feed"]) {
            return;
        }
        if self.loading || self.older_loading {
            self.refresh_again = true;
            return;
        }
        self.refresh_again = false;
        self.generation += 1;
        let generation = self.generation;
        self.loading = true;
        self.older_loading = false;
        self.error = None;
        self.retry_older = false;
        let client = self.client.clone();
        let mission = self.mission.clone();
        let (followed, follow_done) = oneshot::channel();
        self.follow_done = Some(follow_done);
        if self.release.is_none() {
            let (release, released) = oneshot::channel::<Option<oneshot::Receiver<()>>>();
            self.release = Some(release);
            let cleanup_client = client.clone();
            let cleanup_mission = mission.clone();
            let follows = self.follows.clone();
            let owner = self.follow_owner;
            cx.background_spawn(async move {
                if let Ok(Some(done)) = released.await {
                    let _ = done.await;
                }
                // A reopened feed may acquire ownership before this cleanup runs.
                let _request = follows.requests.lock().unwrap();
                if follows.owner.load(Ordering::SeqCst) == owner && cleanup_client.valid() {
                    let _ = cleanup_client.unfollow_feed(cleanup_mission);
                }
            })
            .detach();
        }
        let follows = self.follows.clone();
        let owner = self.follow_owner;
        // Tail appends after failure do not prove the history gap was covered.
        let known_ids = self
            .catch_up_ids
            .get_or_insert_with(|| self.entries.iter().map(|entry| entry.id.clone()).collect())
            .clone();
        let task = cx.background_spawn(async move {
            let result = {
                let _request = follows.requests.lock().unwrap();
                if follows.owner.load(Ordering::SeqCst) == owner {
                    client.follow_feed(mission.clone())
                } else {
                    Err(runner_core::protocol::ClientError::msg(
                        "The mission feed is no longer open.",
                    ))
                }
            };
            let _ = followed.send(());
            result?;
            let mut page = client.feed(mission.clone(), None)?;
            while !known_ids.is_empty()
                && !page
                    .entries
                    .iter()
                    .any(|entry| known_ids.contains(&entry.id))
            {
                let Some(before) = page.before.take() else {
                    break;
                };
                let older = client.feed(mission.clone(), Some(before))?;
                page.entries.extend(older.entries);
                page.before = older.before;
            }
            let overlaps = page
                .entries
                .iter()
                .any(|entry| known_ids.contains(&entry.id));
            Ok::<_, runner_core::protocol::ClientError>((page, overlaps))
        });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |feed: &mut Self, cx| {
                if feed.generation != generation || !feed.client.valid() {
                    return;
                }
                feed.loading = false;
                match result {
                    Ok((page, overlaps)) => {
                        let initial = !feed.history_loaded || feed.entries.is_empty();
                        feed.catch_up_ids = None;
                        feed.history_loaded = true;
                        merge_history(&mut feed.entries, page.entries, false);
                        if initial || !overlaps {
                            feed.before = page.before;
                        }
                    }
                    Err(error) => feed.error = Some(error.to_string()),
                }
                merge_append(&mut feed.entries, std::mem::take(&mut feed.appends));
                if feed.refresh_again {
                    feed.refresh(cx);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn retry(&mut self, cx: &mut Context<Self>) {
        if self.retry_older {
            self.older(cx);
        } else {
            self.refresh(cx);
        }
    }

    pub(crate) fn can_load_older(&self) -> bool {
        !self.loading && !self.older_loading && self.catch_up_ids.is_none() && self.before.is_some()
    }

    pub(crate) fn older(&mut self, cx: &mut Context<Self>) {
        if !self.can_load_older() {
            return;
        }
        let before = self.before.clone().unwrap();
        let client = self.client.clone();
        let mission = self.mission.clone();
        let generation = self.generation;
        self.older_loading = true;
        self.error = None;
        self.retry_older = true;
        let task = cx.background_spawn(async move { client.feed(mission, Some(before)) });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |feed: &mut Self, cx| {
                if feed.generation != generation || !feed.client.valid() {
                    return;
                }
                feed.older_loading = false;
                match result {
                    Ok(FeedPage { entries, before }) => {
                        merge_history(&mut feed.entries, entries, true);
                        feed.before = before;
                    }
                    Err(error) => feed.error = Some(error.to_string()),
                }
                if feed.refresh_again {
                    feed.refresh(cx);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

impl Drop for MissionFeed {
    fn drop(&mut self) {
        self.release_current();
    }
}

fn merge_append(entries: &mut Vec<FeedEntry>, batch: Vec<FeedEntry>) {
    let mut ids = entries
        .iter()
        .map(|entry| entry.id.clone())
        .collect::<HashSet<_>>();
    entries.extend(
        batch
            .into_iter()
            .filter(|entry| ids.insert(entry.id.clone())),
    );
}

fn merge_history(entries: &mut Vec<FeedEntry>, mut newest_first: Vec<FeedEntry>, older: bool) {
    newest_first.reverse();
    let mut position = newest_first
        .iter()
        .find_map(|entry| entries.iter().position(|existing| existing.id == entry.id))
        .unwrap_or(if older { 0 } else { entries.len() });
    for entry in newest_first {
        if let Some(index) = entries.iter().position(|existing| existing.id == entry.id) {
            position = index + 1;
        } else {
            entries.insert(position, entry);
            position += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reopening_before_old_cleanup_keeps_the_new_feed_followed() {
        use crate::surfaces::remote_client::tests as remote_test;
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        let temp = tempfile::tempdir().unwrap();
        let mut cx = gpui::TestAppContext::single();
        let (store, fake, clients) = remote_test::fixture(&mut cx, temp.path());
        remote_test::enable_missions(&fake);
        let window = remote_test::add_window(&mut cx, &store, temp.path(), "feed-reopen");
        window
            .update(&mut cx, |root, window, cx| {
                remote_test::choose_host(root, window, cx)
            })
            .unwrap();
        cx.run_until_parked();
        let client = window
            .read_with(&cx, |root, cx| root.front_page_client(cx))
            .unwrap();
        let first = clients.update(&mut cx, |clients, cx| {
            clients.mission_feed(client.clone(), "remote-mission".into(), cx)
        });
        cx.run_until_parked();
        let (release, followed) = first.update(&mut cx, |feed, _| {
            (feed.release.take().unwrap(), feed.follow_done.take())
        });
        cx.update(|_| drop(first));
        cx.run_until_parked();
        let reopened = clients.update(&mut cx, |clients, cx| {
            clients.mission_feed(client, "remote-mission".into(), cx)
        });
        cx.run_until_parked();
        assert_eq!(fake.request_count("follow"), 2);
        release.send(followed).unwrap();
        cx.run_until_parked();
        assert_eq!(fake.request_count("unfollow"), 0);
        cx.update(|_| drop(reopened));
        cx.run_until_parked();
        assert_eq!(fake.request_count("unfollow"), 1);
    }

    fn entry(id: &str) -> FeedEntry {
        serde_json::from_value(serde_json::json!({"id":id,"ts":"2026-10-24T05:00:00Z","from":"coder","to":[],"kind":"message","text":id})).unwrap()
    }
    fn ids(entries: &[FeedEntry]) -> Vec<&str> {
        entries.iter().map(|entry| entry.id.as_str()).collect()
    }
    #[test]
    fn history_and_append_orders_preserve_overlap_older_pages_and_append_order() {
        for event_first in [true, false] {
            let mut entries = Vec::new();
            let mut pending = Vec::new();
            if event_first {
                merge_append(&mut pending, vec![entry("c"), entry("d")]);
            }
            merge_history(
                &mut entries,
                vec![entry("c"), entry("b"), entry("a")],
                false,
            );
            if !event_first {
                merge_append(&mut pending, vec![entry("c"), entry("d")]);
            }
            merge_append(&mut entries, pending);
            assert_eq!(ids(&entries), ["a", "b", "c", "d"]);
            merge_history(
                &mut entries,
                vec![entry("a"), entry("older-b"), entry("older-a")],
                true,
            );
            assert_eq!(ids(&entries), ["older-a", "older-b", "a", "b", "c", "d"]);
            merge_history(&mut entries, vec![entry("f"), entry("e")], false);
            assert_eq!(
                ids(&entries),
                ["older-a", "older-b", "a", "b", "c", "d", "e", "f"]
            );
            merge_append(
                &mut entries,
                vec![entry("append-z"), entry("append-q"), entry("append-z")],
            );
            assert_eq!(&ids(&entries)[8..], ["append-z", "append-q"]);
        }
    }
}
