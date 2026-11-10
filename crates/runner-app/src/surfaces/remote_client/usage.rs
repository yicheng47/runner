use super::{model::*, *};
use runner_core::{
    companion::Usage,
    protocol::{model::Runtime, usage::UsageSnapshot},
};
use std::time::{Duration, Instant};

#[derive(Default)]
pub(super) struct UsagePolling {
    views: usize,
    popovers: usize,
    opened: bool,
    last_request: Option<Instant>,
    in_flight: bool,
}

impl UsagePolling {
    pub(super) fn reconnected(&mut self) {
        self.last_request = None;
    }
    fn due(&self, now: Instant, refreshing: bool) -> bool {
        self.views > 0
            && !self.in_flight
            && (self.opened
                || self.last_request.is_none_or(|last| {
                    now.duration_since(last)
                        >= if self.popovers > 0 && refreshing {
                            Duration::from_secs(2)
                        } else {
                            Duration::from_secs(60)
                        }
                }))
    }
}

pub(crate) struct UsageWatch {
    computer: String,
    popover: bool,
    commands: futures::channel::mpsc::UnboundedSender<Command>,
}

impl Drop for UsageWatch {
    fn drop(&mut self) {
        let _ = self.commands.unbounded_send(Command::UsageWatch(
            self.computer.clone(),
            self.popover,
            false,
        ));
    }
}

pub(crate) fn usage_snapshot(usage: &Usage) -> UsageSnapshot {
    UsageSnapshot {
        runtimes: usage
            .agents
            .iter()
            .filter_map(|agent| {
                let runtime = Runtime::ALL
                    .into_iter()
                    .find(|runtime| runtime.key() == agent.name)?;
                Some((
                    runtime,
                    runner_core::protocol::usage::RuntimeUsage {
                        value: agent.value.clone().map(Into::into),
                        error: agent.error.clone().map(Into::into),
                    },
                ))
            })
            .collect(),
        refreshing: usage.refreshing,
        ..Default::default()
    }
}

impl ClientState {
    pub(crate) fn watch_usage(&self, computer: String, popover: bool) -> UsageWatch {
        self.send(Command::UsageWatch(computer.clone(), popover, true));
        UsageWatch {
            computer,
            popover,
            commands: self.commands.clone(),
        }
    }

    pub(super) fn update_usage_watch(&mut self, computer: String, popover: bool, acquire: bool) {
        let polling = self.usage_polling.entry(computer).or_default();
        let count = if popover {
            &mut polling.popovers
        } else {
            &mut polling.views
        };
        if acquire {
            *count += 1;
            if popover {
                polling.opened = true;
            } else if *count == 1 {
                polling.last_request = None;
            }
        } else {
            *count = count.saturating_sub(1);
        }
    }

    pub(super) fn poll_usage(&mut self, cx: &mut Context<Self>) {
        let now = cx.background_executor().now();
        let computers: Vec<_> = self
            .usage_polling
            .iter()
            .filter_map(|(computer, polling)| {
                (self.connection(computer) == Connection::Connected
                    && polling.due(
                        now,
                        self.usage
                            .get(computer)
                            .is_some_and(|usage| usage.refreshing),
                    ))
                .then_some(computer.clone())
            })
            .collect();
        for computer in computers {
            let Some(access) = self
                .capabilities
                .get(&computer)
                .filter(|access| access.supports(&["usage"]))
            else {
                continue;
            };
            let client = crate::surfaces::front_page::FrontPageClient::remote(
                self.client.clone(),
                access.clone(),
            );
            let target = client.target.clone();
            let polling = self.usage_polling.get_mut(&computer).unwrap();
            let popover = std::mem::take(&mut polling.opened);
            polling.last_request = Some(now);
            polling.in_flight = true;
            let task = cx.background_spawn(async move { client.usage(popover) });
            cx.spawn(async move |weak, cx| {
                let result = task.await;
                let _ = weak.update(cx, |state, cx| {
                    if let Some(polling) = state.usage_polling.get_mut(&computer) {
                        polling.in_flight = false;
                    }
                    if state
                        .capabilities
                        .get(&computer)
                        .is_some_and(|access| access.valid() && access.target == target)
                    {
                        if let Ok(usage) = result {
                            state.usage.insert(computer, usage);
                        }
                    }
                    state.poll_usage(cx);
                    cx.notify();
                });
            })
            .detach();
        }
    }
}

impl RemoteClient {
    pub(crate) fn remote_usage(&self, cx: &App) -> Option<(Usage, Connection, String)> {
        let remote = self.workspace.as_ref()?;
        let clients = state(&self.app_store, cx);
        let clients = clients.read(cx);
        if clients
            .capabilities
            .get(&remote.computer.id)
            .is_some_and(|access| access.lacks("usage"))
        {
            return None;
        }
        Some((
            clients.usage.get(&remote.computer.id)?.clone(),
            remote.connection,
            remote.computer.name.clone(),
        ))
    }

    /// The connected host's name and Runner version when it cannot send usage.
    pub(crate) fn older_host(&self, cx: &App) -> Option<(String, String)> {
        let remote = self.workspace.as_ref()?;
        let lacks = remote.connection == Connection::Connected
            && state(&self.app_store, cx)
                .read(cx)
                .capabilities
                .get(&remote.computer.id)
                .is_some_and(|access| access.lacks("usage"));
        lacks.then(|| {
            (
                remote.computer.name.clone(),
                remote.computer.info.runner_version.clone(),
            )
        })
    }

    pub(crate) fn sync_usage_popover(&mut self, open: bool, cx: &App) {
        let computer = self
            .workspace
            .as_ref()
            .filter(|_| open)
            .map(|remote| &remote.computer.id);
        if self.usage_popover.as_ref().map(|watch| &watch.computer) != computer {
            self.usage_popover = computer.map(|computer| {
                state(&self.app_store, cx)
                    .read(cx)
                    .watch_usage(computer.clone(), true)
            });
        }
    }
}
