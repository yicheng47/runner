use super::{persistence::Saved, store::Secrets, transport::Failure};
use chrono::{DateTime, Duration, Utc};
use runner_core::remote::{self, *};

#[derive(Debug)]
pub enum Command {
    #[cfg(test)]
    TestDeadline,
    #[cfg(test)]
    TestClaimDeadline,
    #[cfg(test)]
    TestTextAtClaimDeadline(String),
    Start {
        origin: Option<String>,
        confirm_account: bool,
        key: HostKey,
        owner: Option<u64>,
    },
    Confirm(bool),
    Cancel,
    Pause(bool),
    Retry,
    Logout,
    OwnerGone(u64),
    Shutdown,
}
#[derive(Debug)]
pub enum Input {
    Command(Command),
    Started(Result<StartResponse, Failure>),
    Polled(Result<PollResponse, Failure>),
    Failure(Failure),
    Welcome {
        host_id: String,
        name: String,
        account: Account,
    },
    Text(String),
    Frame,
    Deadline,
}
#[derive(Debug)]
pub enum Effect {
    Log(String),
    Start(StartRequest),
    Poll(PollRequest),
    Connect,
    Close,
    Store,
    Save,
    Delete(String),
    SignOut {
        origin: String,
        credential: Secret,
        name: String,
        account: String,
    },
}
struct Enrollment {
    key: HostKey,
    device: Secret,
    confirm: bool,
    owner: Option<u64>,
    interval: i64,
    recovery_end: DateTime<Utc>,
    start_deadline: DateTime<Utc>,
}
#[derive(Default)]
pub struct Machine {
    pub host: HostInfo,
    pub status: RemoteStatus,
    pub saved: Option<Saved>,
    pub secrets: Option<Secrets>,
    enrollment: Option<Enrollment>,
    pub deadline: Option<DateTime<Utc>>,
    attempt: u32,
    welcome_at: Option<DateTime<Utc>>,
    last_frame: Option<DateTime<Utc>>,
    poll_attempt: u32,
    pub shutting_down: bool,
}
impl Machine {
    pub fn restore(
        saved: Option<Saved>,
        secrets: Option<Secrets>,
        now: DateTime<Utc>,
    ) -> (Self, Vec<Effect>) {
        let mut m = Self {
            saved,
            secrets,
            ..Self::default()
        };
        let mut effects = Vec::new();
        if let Some(s) = &m.saved {
            m.status.origin = s.origin.clone();
            m.status.account = Some(s.account.clone());
            m.status.host_id = Some(s.host_id.clone());
            m.status.host_name = Some(s.host_name.clone());
            m.status.fingerprint = Some(s.fingerprint.clone());
            if m.secrets.is_none() {
                m.saved = None;
                m.status.host_id = None;
                m.status.state = RemoteState::Stopped {
                    reason: StopReason::Removed,
                };
                effects.push(Effect::Delete(m.status.origin.clone()));
            } else if !s.activated && now >= s.claim_expires_at {
                m.expire(&mut effects);
            } else if let Some(reason) = s.stopped {
                m.status.state = RemoteState::Stopped { reason };
                m.deadline = (!s.activated).then_some(s.claim_expires_at);
            } else if s.paused {
                m.status.state = RemoteState::Paused;
                m.deadline = (!s.activated).then_some(s.claim_expires_at);
            } else {
                m.connect(now, &mut effects);
            }
        }
        (m, effects)
    }
    pub fn step(
        &mut self,
        input: Input,
        now: DateTime<Utc>,
        rng: &mut impl FnMut() -> u64,
    ) -> Result<(Vec<Effect>, Option<DateTime<Utc>>), String> {
        let mut effects = Vec::new();
        let mut input = input;
        let failure = match &mut input {
            Input::Started(Err(f)) | Input::Polled(Err(f)) | Input::Failure(f) => Some(f),
            _ => None,
        };
        if let Some(failure) = failure {
            failure.message = remote::redact(&failure.message);
            if let Some(e) = &self.enrollment {
                if !e.device.0.is_empty() {
                    failure.message = failure.message.replace(&e.device.0, "[redacted]");
                }
            }
            effects.push(Effect::Log(format!("remote: {}", failure.message)));
        }
        if matches!(input, Input::Started(_) | Input::Deadline)
            && self.start_deadline().is_some_and(|at| now >= at)
        {
            let host = reqwest::Url::parse(&self.status.origin)
                .ok()
                .and_then(|url| url.host_str().map(str::to_owned))
                .unwrap_or_else(|| self.status.origin.clone());
            self.off();
            self.status.last_error = Some(format!("Couldn't reach {}.", remote::redact(&host)));
            return Ok((effects, self.deadline));
        }
        if self
            .saved
            .as_ref()
            .is_some_and(|s| !s.activated && now >= s.claim_expires_at)
            && !matches!(input, Input::Command(Command::Logout | Command::Shutdown))
        {
            self.expire(&mut effects);
            return Ok((effects, self.deadline));
        }
        match input {
            Input::Command(command) => {
                match command {
                    #[cfg(test)]
                    Command::TestDeadline => return self.step(Input::Deadline, now, rng),
                    #[cfg(test)]
                    Command::TestClaimDeadline => {
                        return self.step(
                            Input::Deadline,
                            self.saved
                                .as_ref()
                                .map(|s| s.claim_expires_at)
                                .unwrap_or(now),
                            rng,
                        )
                    }
                    #[cfg(test)]
                    Command::TestTextAtClaimDeadline(text) => {
                        return self.step(
                            Input::Text(text),
                            self.saved
                                .as_ref()
                                .map(|s| s.claim_expires_at)
                                .unwrap_or(now),
                            rng,
                        )
                    }
                    Command::Start {
                        origin,
                        confirm_account,
                        key,
                        owner,
                    } => {
                        if !matches!(
                            self.status.state,
                            RemoteState::Off
                                | RemoteState::Stopped { .. }
                                | RemoteState::Enrolling { .. }
                        ) {
                            return Err("This computer is already connected. Run `runner remote logout` first.".into());
                        }
                        let origin =
                            canonical_origin(origin.as_deref().unwrap_or(&self.status.origin))
                                .map_err(str::to_owned)?;
                        if self.secrets.is_some() {
                            self.logout(&mut effects);
                        }
                        self.status = RemoteStatus {
                            origin,
                            fingerprint: Some(fingerprint(&key.public)),
                            ..RemoteStatus::default()
                        };
                        effects.push(Effect::Start(StartRequest {
                            host: self.host.clone(),
                            host_key: encode_key(&key.public),
                        }));
                        self.enrollment = Some(Enrollment {
                            key,
                            device: Secret::default(),
                            confirm: confirm_account,
                            owner,
                            interval: 5,
                            recovery_end: now,
                            start_deadline: now + Duration::seconds(30),
                        });
                        self.deadline = None;
                        self.poll_attempt = 0;
                    }
                    Command::Confirm(allow) => {
                        if !matches!(self.status.state, RemoteState::Confirming { .. }) {
                            return Err("No account is waiting for confirmation.".into());
                        }
                        if allow {
                            self.enrollment = None;
                            effects.push(Effect::Store);
                            self.connect(now, &mut effects);
                        } else {
                            self.logout(&mut effects);
                        }
                    }
                    Command::Cancel => {
                        if !matches!(self.status.state, RemoteState::Enrolling { .. }) {
                            return Err("No enrollment is waiting for approval.".into());
                        }
                        self.off();
                    }
                    Command::OwnerGone(owner) => {
                        if self
                            .enrollment
                            .as_ref()
                            .is_some_and(|e| e.confirm && e.owner == Some(owner))
                        {
                            if self.secrets.is_some() {
                                self.logout(&mut effects);
                            } else {
                                self.off();
                            }
                        }
                    }
                    Command::Pause(paused) => {
                        let s = self
                            .saved
                            .as_mut()
                            .ok_or("This computer is not connected.")?;
                        if matches!(
                            self.status.state,
                            RemoteState::Confirming { .. } | RemoteState::Stopped { .. }
                        ) {
                            return Err("Remote access cannot be paused in this state.".into());
                        }
                        s.paused = paused;
                        effects.push(Effect::Save);
                        if paused {
                            self.status.state = RemoteState::Paused;
                            self.deadline = (!s.activated).then_some(s.claim_expires_at);
                            effects.push(Effect::Close);
                        } else {
                            self.connect(now, &mut effects);
                        }
                    }
                    Command::Retry => {
                        if !matches!(
                            self.status.state,
                            RemoteState::Reconnecting { .. }
                                | RemoteState::Stopped {
                                    reason: StopReason::Removed | StopReason::InUseElsewhere
                                }
                        ) || self.secrets.is_none()
                        {
                            return Err("Remote access cannot be retried in this state.".into());
                        }
                        if let Some(s) = &mut self.saved {
                            s.stopped = None;
                            effects.push(Effect::Save);
                        }
                        self.connect(now, &mut effects);
                    }
                    Command::Logout => {
                        self.logout(&mut effects);
                    }
                    Command::Shutdown => {
                        effects.push(Effect::Close);
                        self.shutting_down = true;
                        self.deadline = None;
                    }
                }
            }
            Input::Started(result) => match result {
                Ok(response) => {
                    if !same_origin(&self.status.origin, &response.verification_uri)
                        || !same_origin(&self.status.origin, &response.verification_uri_complete)
                        || response.device_code.0.is_empty()
                        || response.user_code.is_empty()
                        || response.expires_in <= 0
                        || response.interval <= 0
                    {
                        self.stop(StopReason::UpdateRequired, &mut effects);
                    } else if let Some(e) = &mut self.enrollment {
                        let expires_at = now + Duration::seconds(response.expires_in.into());
                        e.device = response.device_code;
                        e.interval = response.interval.into();
                        e.recovery_end = expires_at + Duration::minutes(5);
                        self.status.state = RemoteState::Enrolling {
                            user_code: response.user_code,
                            verification_uri: response.verification_uri,
                            verification_uri_complete: response.verification_uri_complete,
                            expires_at,
                        };
                        self.deadline = Some(now + Duration::seconds(e.interval));
                    }
                }
                Err(f) => {
                    if f.code == "resource_exhausted" {
                        self.off();
                        self.status.last_error =
                            Some("Too many attempts. Try again in a minute.".into());
                    } else if f.code == "invalid_argument" {
                        self.stop(StopReason::UpdateRequired, &mut effects);
                    } else {
                        self.status.last_error = Some(f.message);
                        self.deadline = self.start_deadline().map(|at| {
                            (now + Duration::milliseconds(backoff(self.poll_attempt, rng())))
                                .min(at)
                        });
                        self.poll_attempt = self.poll_attempt.saturating_add(1);
                    }
                }
            },
            Input::Polled(result) => match result {
                Ok(response) => match response.state {
                    EnrollmentState::Pending | EnrollmentState::Unspecified => {
                        self.poll_later(now, 0)
                    }
                    EnrollmentState::Denied => self.stop(StopReason::Denied, &mut effects),
                    EnrollmentState::Expired => self.stop(StopReason::Expired, &mut effects),
                    EnrollmentState::Approved => {
                        let claim = response
                            .claim_expires_at
                            .as_deref()
                            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                            .map(|d| d.with_timezone(&Utc));
                        if !valid_credential(&response.credential.0)
                            || !valid_relay_url(&self.status.origin, &response.relay_url)
                            || response.host_id.is_empty()
                            || response.account.is_none()
                            || claim.is_none()
                        {
                            self.stop(StopReason::UpdateRequired, &mut effects);
                        } else if let (Some(e), Some(account), Some(claim)) =
                            (self.enrollment.as_ref(), response.account, claim)
                        {
                            self.secrets = Some(Secrets {
                                v: 1,
                                key: Secret(encode_key(&e.key.private)),
                                credential: response.credential,
                            });
                            let account: RemoteAccount = account.into();
                            self.saved = Some(Saved {
                                version: 1,
                                origin: self.status.origin.clone(),
                                host_id: response.host_id.clone(),
                                relay_url: response.relay_url,
                                account: account.clone(),
                                host_name: String::new(),
                                fingerprint: self.status.fingerprint.clone().unwrap_or_default(),
                                activated: false,
                                claim_expires_at: claim,
                                paused: false,
                                stopped: None,
                            });
                            self.status.account = Some(account.clone());
                            self.status.host_id = Some(response.host_id);
                            if claim <= now {
                                self.expire(&mut effects);
                            } else if e.confirm {
                                self.status.state = RemoteState::Confirming {
                                    account,
                                    expires_at: claim,
                                };
                                self.deadline = Some(claim);
                            } else {
                                self.enrollment = None;
                                effects.push(Effect::Store);
                                self.connect(now, &mut effects);
                            }
                        }
                    }
                },
                Err(f) => {
                    if f.code == "invalid_argument" {
                        self.status.last_error = Some(f.message);
                        self.stop(StopReason::UpdateRequired, &mut effects);
                    } else {
                        if f.code == "resource_exhausted" {
                            if let Some(e) = &mut self.enrollment {
                                e.interval += 5;
                            }
                        }
                        self.status.last_error = Some(f.message);
                        if f.code == "resource_exhausted" {
                            self.poll_later(now, 0);
                        } else {
                            self.poll_later(now, backoff(self.poll_attempt, rng()));
                            self.poll_attempt = self.poll_attempt.saturating_add(1);
                        }
                    }
                }
            },
            Input::Failure(f) => {
                self.status.last_error = Some(f.message);
                if matches!(f.retry, Retry::Stop(StopReason::Removed))
                    && self.saved.as_ref().is_some_and(|s| !s.activated)
                {
                    self.expire(&mut effects);
                } else if let Retry::Stop(reason) = f.retry {
                    self.stop(reason, &mut effects);
                } else {
                    effects.push(Effect::Close);
                    if self
                        .welcome_at
                        .is_some_and(|at| now.signed_duration_since(at) >= Duration::seconds(60))
                    {
                        self.attempt = 0;
                    }
                    let delay = match f.retry {
                        Retry::After(s) => (s.min(i64::MAX as u64 / 1000) * 1000) as i64,
                        Retry::Restart => 1000 + (rng() % 9001) as i64,
                        _ => backoff(self.attempt, rng()),
                    };
                    if f.retry == Retry::Backoff {
                        self.attempt = self.attempt.saturating_add(1);
                    }
                    let at = now
                        .checked_add_signed(Duration::milliseconds(delay))
                        .unwrap_or(DateTime::<Utc>::MAX_UTC);
                    effects.push(Effect::Log(format!(
                        "remote: retry in {delay} ms, attempt {}",
                        self.attempt
                    )));
                    self.status.state = RemoteState::Reconnecting {
                        attempt: self.attempt,
                        next_retry_at: at,
                    };
                    self.deadline = Some(at);
                    self.welcome_at = None;
                    self.last_frame = None;
                    self.bound_claim_deadline();
                }
            }
            Input::Welcome {
                host_id,
                name,
                account,
            } => {
                if self.saved.as_ref().is_none_or(|s| s.host_id != host_id) {
                    return self.step(Input::Failure(Failure::protocol()), now, rng);
                }
                if let Some(s) = &mut self.saved {
                    s.host_name = remote::redact(&name);
                    s.account = account.into();
                    s.activated = true;
                    self.status.host_name = Some(s.host_name.clone());
                    self.status.account = Some(s.account.clone());
                }
                self.status.state = RemoteState::Connected { since: now };
                self.status.last_error = None;
                self.welcome_at = Some(now);
                self.last_frame = Some(now);
                self.deadline = Some(now + Duration::seconds(75));
                effects.push(Effect::Save);
            }
            Input::Text(text) => {
                let input = match serde_json::from_str::<Control>(&text) {
                    Ok(Control::Welcome {
                        host_id,
                        name,
                        account,
                    }) => Input::Welcome {
                        host_id,
                        name,
                        account,
                    },
                    Ok(_) => Input::Frame,
                    Err(_) => Input::Failure(Failure::protocol()),
                };
                return self.step(input, now, rng);
            }
            Input::Frame => {
                self.last_frame = Some(now);
                if matches!(self.status.state, RemoteState::Connected { .. }) {
                    self.deadline = Some(now + Duration::seconds(75));
                }
            }
            Input::Deadline => match self.status.state {
                RemoteState::Enrolling { .. } => {
                    if self
                        .enrollment
                        .as_ref()
                        .is_some_and(|e| now >= e.recovery_end)
                    {
                        self.stop(StopReason::Expired, &mut effects);
                    } else if let Some(e) = &self.enrollment {
                        effects.push(Effect::Poll(PollRequest {
                            device_code: e.device.clone(),
                        }));
                        self.deadline = None;
                    }
                }
                RemoteState::Confirming { .. } => self.expire(&mut effects),
                RemoteState::Reconnecting { .. } => self.connect(now, &mut effects),
                RemoteState::Connecting { .. } | RemoteState::Connected { .. } => {
                    return self.step(Input::Failure(Failure::network()), now, rng)
                }
                _ => {
                    if let Some(e) = &self.enrollment {
                        if e.device.0.is_empty() {
                            effects.push(Effect::Start(StartRequest {
                                host: self.host.clone(),
                                host_key: encode_key(&e.key.public),
                            }));
                            self.deadline = None;
                        }
                    }
                }
            },
        }
        Ok((effects, self.deadline))
    }
    pub fn start_deadline(&self) -> Option<DateTime<Utc>> {
        self.enrollment
            .as_ref()
            .filter(|e| e.device.0.is_empty())
            .map(|e| e.start_deadline)
    }
    fn poll_later(&mut self, now: DateTime<Utc>, extra: i64) {
        if let Some(e) = &self.enrollment {
            self.deadline = Some(
                (now + Duration::seconds(e.interval) + Duration::milliseconds(extra))
                    .min(e.recovery_end),
            );
        }
    }
    pub fn connect(&mut self, now: DateTime<Utc>, effects: &mut Vec<Effect>) {
        self.status.state = RemoteState::Connecting {
            attempt: self.attempt,
        };
        self.deadline = Some(now + Duration::seconds(10));
        self.bound_claim_deadline();
        effects.push(Effect::Connect);
    }
    fn bound_claim_deadline(&mut self) {
        if let Some(s) = &self.saved {
            if !s.activated {
                self.deadline = Some(
                    self.deadline
                        .map_or(s.claim_expires_at, |d| d.min(s.claim_expires_at)),
                );
            }
        }
    }
    fn stop(&mut self, reason: StopReason, effects: &mut Vec<Effect>) {
        self.status.state = RemoteState::Stopped { reason };
        self.deadline = None;
        self.enrollment = None;
        effects.push(Effect::Close);
        if let Some(s) = &mut self.saved {
            s.stopped = Some(reason);
            effects.push(Effect::Save);
        }
        self.bound_claim_deadline();
    }
    fn expire(&mut self, effects: &mut Vec<Effect>) {
        effects.extend([Effect::Close, Effect::Delete(self.status.origin.clone())]);
        self.saved = None;
        self.secrets = None;
        self.stop(StopReason::Expired, effects);
    }
    fn off(&mut self) {
        let origin = self.status.origin.clone();
        self.status = RemoteStatus {
            origin,
            ..RemoteStatus::default()
        };
        self.enrollment = None;
        self.saved = None;
        self.secrets = None;
        self.deadline = None;
        self.attempt = 0;
        self.welcome_at = None;
        self.last_frame = None;
    }
    fn logout(&mut self, effects: &mut Vec<Effect>) {
        effects.push(Effect::Close);
        if let Some(s) = &self.secrets {
            effects.push(Effect::Delete(self.status.origin.clone()));
            effects.push(Effect::SignOut {
                origin: self.status.origin.clone(),
                credential: s.credential.clone(),
                name: self
                    .status
                    .host_name
                    .clone()
                    .unwrap_or_else(|| "This computer".into()),
                account: self
                    .status
                    .account
                    .as_ref()
                    .map(|a| a.username.clone())
                    .unwrap_or_default(),
            });
        }
        self.off();
    }
}
pub fn backoff(attempt: u32, random: u64) -> i64 {
    (random % ((1000u64 << attempt.min(6)).min(60_000) + 1)) as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    pub(super) fn now() -> DateTime<Utc> {
        DateTime::from_timestamp(1_800_000_000, 0).unwrap()
    }
    pub(super) fn command(m: &mut Machine, command: Command, at: DateTime<Utc>) -> Vec<Effect> {
        m.step(Input::Command(command), at, &mut || 9000).unwrap().0
    }
    pub(super) fn begin(confirm: bool) -> Machine {
        let mut m = Machine::default();
        command(
            &mut m,
            Command::Start {
                origin: Some("https://runner.example".into()),
                confirm_account: confirm,
                key: HostKey::generate().unwrap(),
                owner: Some(0),
            },
            now(),
        );
        let response = serde_json::from_str(include_str!(
            "../../../runner-core/src/remote/fixtures/start_response.json"
        ))
        .unwrap();
        m.step(Input::Started(Ok(response)), now(), &mut || 0)
            .unwrap();
        m
    }
    pub(super) fn approve(m: &mut Machine) -> Vec<Effect> {
        let mut response: PollResponse = serde_json::from_str(include_str!(
            "../../../runner-core/src/remote/fixtures/poll_approved.json"
        ))
        .unwrap();
        response.claim_expires_at = Some((now() + Duration::minutes(12)).to_rfc3339());
        m.step(Input::Polled(Ok(response)), now(), &mut || 0)
            .unwrap()
            .0
    }
    fn welcome(m: &mut Machine, at: DateTime<Utc>) {
        m.step(
            Input::Welcome {
                host_id: "7383920468475904".into(),
                name: "Mac Studio".into(),
                account: Account {
                    username: "jason".into(),
                    display_name: "Jason".into(),
                },
            },
            at,
            &mut || 0,
        )
        .unwrap();
    }
    #[test]
    fn starting_stops_after_thirty_seconds_and_a_new_attempt_gets_a_fresh_budget() {
        for confirm_account in [false, true] {
            let mut m = Machine::default();
            command(
                &mut m,
                Command::Start {
                    origin: Some("https://runner.example".into()),
                    confirm_account,
                    key: HostKey::generate().unwrap(),
                    owner: Some(0),
                },
                now(),
            );
            let end = now() + Duration::seconds(30);
            assert_eq!(m.start_deadline(), Some(end));
            m.step(Input::Started(Err(Failure::network())), now(), &mut || 500)
                .unwrap();
            let retry_at = m.deadline.unwrap();
            assert_eq!(retry_at, now() + Duration::milliseconds(500));
            assert!(matches!(
                m.step(Input::Deadline, retry_at, &mut || 0)
                    .unwrap()
                    .0
                    .as_slice(),
                [Effect::Start(_)]
            ));
            m.step(
                Input::Started(Err(Failure::network())),
                end - Duration::milliseconds(100),
                &mut || 500,
            )
            .unwrap();
            assert_eq!(m.deadline, Some(end));
            let (effects, deadline) = m.step(Input::Deadline, end, &mut || 0).unwrap();
            assert!(effects.is_empty());
            assert_eq!(deadline, None);
            assert!(matches!(m.status.state, RemoteState::Off));
            assert_eq!(
                m.status.last_error.as_deref(),
                Some("Couldn't reach runner.example.")
            );
            assert!(m.status.fingerprint.is_none());
            assert!(m.enrollment.is_none());
            let next = end + Duration::seconds(1);
            command(
                &mut m,
                Command::Start {
                    origin: None,
                    confirm_account,
                    key: HostKey::generate().unwrap(),
                    owner: Some(0),
                },
                next,
            );
            assert_eq!(m.start_deadline(), Some(next + Duration::seconds(30)));
            assert!(m.status.last_error.is_none());
            let response = serde_json::from_str(include_str!(
                "../../../runner-core/src/remote/fixtures/start_response.json"
            ))
            .unwrap();
            m.step(Input::Started(Ok(response)), next, &mut || 0)
                .unwrap();
            assert!(matches!(m.status.state, RemoteState::Enrolling { .. }));
            assert_eq!(m.start_deadline(), None);
        }
    }
    #[test]
    fn app_cli_confirmation_refusal_cancel_and_single_enrollment() {
        let mut app = begin(false);
        assert!(matches!(app.status.state, RemoteState::Enrolling { .. }));
        assert!(matches!(approve(&mut app)[0], Effect::Store));
        assert!(matches!(app.status.state, RemoteState::Connecting { .. }));
        assert!(app
            .step(
                Input::Command(Command::Start {
                    origin: None,
                    confirm_account: false,
                    key: HostKey::generate().unwrap(),
                    owner: Some(0)
                }),
                now(),
                &mut || 0
            )
            .is_err());
        welcome(&mut app, now());
        assert!(matches!(app.status.state, RemoteState::Connected { .. }));
        let mut cli = begin(true);
        approve(&mut cli);
        assert!(matches!(cli.status.state, RemoteState::Confirming { .. }));
        let effects = command(&mut cli, Command::Confirm(false), now());
        assert!(matches!(
            effects.as_slice(),
            [Effect::Close, Effect::Delete(_), Effect::SignOut { .. }]
        ));
        assert!(matches!(cli.status.state, RemoteState::Off));
        assert!(cli.secrets.is_none());
        let mut cli = begin(true);
        approve(&mut cli);
        command(&mut cli, Command::OwnerGone(0), now());
        assert!(matches!(cli.status.state, RemoteState::Off));
        let mut cli = begin(true);
        approve(&mut cli);
        command(&mut cli, Command::Confirm(true), now());
        assert!(matches!(cli.status.state, RemoteState::Connecting { .. }));
        let mut m = begin(false);
        command(&mut m, Command::Cancel, now());
        assert!(matches!(m.status.state, RemoteState::Off));
    }
    #[test]
    fn backoff_bounds_reset_and_restart_do_not_grow() {
        for (i, cap) in [1000, 2000, 4000, 8000, 16000, 32000, 60000, 60000]
            .into_iter()
            .enumerate()
        {
            assert_eq!(backoff(i as u32, cap), cap as i64);
            assert_eq!(backoff(i as u32, 0), 0);
        }
        let mut m = begin(false);
        approve(&mut m);
        m.attempt = 5;
        welcome(&mut m, now());
        m.step(
            Input::Failure(Failure::network()),
            now() + Duration::seconds(59),
            &mut || 1234,
        )
        .unwrap();
        assert_eq!(m.attempt, 6);
        welcome(&mut m, now());
        m.step(
            Input::Failure(Failure::network()),
            now() + Duration::seconds(60),
            &mut || 1000,
        )
        .unwrap();
        assert_eq!(m.attempt, 1);
        let attempt = m.attempt;
        for random in [0, 9000] {
            let f = Failure {
                retry: Retry::Restart,
                ..Failure::network()
            };
            m.step(Input::Failure(f), now(), &mut || random).unwrap();
            assert_eq!(m.attempt, attempt);
            assert_eq!(
                m.deadline,
                Some(now() + Duration::milliseconds(1000 + random as i64))
            );
        }
    }
    #[test]
    fn heartbeat_welcome_timeout_pause_restore_and_revocation() {
        let mut m = begin(false);
        approve(&mut m);
        assert_eq!(m.deadline, Some(now() + Duration::seconds(10)));
        welcome(&mut m, now());
        assert_eq!(m.deadline, Some(now() + Duration::seconds(75)));
        m.step(Input::Frame, now() + Duration::seconds(25), &mut || 0)
            .unwrap();
        assert_eq!(m.deadline, Some(now() + Duration::seconds(100)));
        m.step(Input::Deadline, now() + Duration::seconds(100), &mut || 0)
            .unwrap();
        assert!(matches!(m.status.state, RemoteState::Reconnecting { .. }));
        let key = m.secrets.as_ref().unwrap().key.0.clone();
        command(&mut m, Command::Pause(true), now());
        assert!(matches!(m.status.state, RemoteState::Paused));
        command(&mut m, Command::Pause(false), now());
        assert_eq!(m.secrets.as_ref().unwrap().key.0, key);
        for code in [4001, 4003] {
            m.step(
                Input::Failure(Failure {
                    retry: close_action(code),
                    ..Failure::network()
                }),
                now(),
                &mut || 0,
            )
            .unwrap();
            assert!(m.secrets.is_some());
            assert!(matches!(m.status.state, RemoteState::Stopped { .. }));
            command(&mut m, Command::Retry, now());
            assert!(matches!(m.status.state, RemoteState::Connecting { .. }));
        }
        let (restored, effects) = Machine::restore(m.saved.clone(), m.secrets.clone(), now());
        assert!(matches!(
            restored.status.state,
            RemoteState::Connecting { .. }
        ));
        assert!(matches!(effects[0], Effect::Connect));
        let (restored, effects) = Machine::restore(m.saved.clone(), None, now());
        assert!(matches!(
            restored.status.state,
            RemoteState::Stopped {
                reason: StopReason::Removed
            }
        ));
        assert!(matches!(effects[0], Effect::Delete(_)));
        let effects = command(&mut m, Command::Logout, now());
        assert!(matches!(
            effects.as_slice(),
            [Effect::Close, Effect::Delete(_), Effect::SignOut { .. }]
        ));
        let old = key;
        command(
            &mut m,
            Command::Start {
                origin: None,
                confirm_account: false,
                key: HostKey::generate().unwrap(),
                owner: Some(0),
            },
            now(),
        );
        assert_ne!(encode_key(&m.enrollment.as_ref().unwrap().key.private), old);
    }
    #[test]
    fn claim_deadlines_poll_slowdown_and_recovery() {
        let mut m = begin(true);
        assert_eq!(m.deadline, Some(now() + Duration::seconds(5)));
        m.step(
            Input::Polled(Err(Failure {
                code: "resource_exhausted".into(),
                ..Failure::network()
            })),
            now(),
            &mut || 0,
        )
        .unwrap();
        assert_eq!(m.deadline, Some(now() + Duration::seconds(10)));
        m.step(
            Input::Polled(Err(Failure::network())),
            now() + Duration::seconds(600),
            &mut || 0,
        )
        .unwrap();
        assert!(m.deadline.unwrap() > now() + Duration::seconds(600));
        m.step(Input::Deadline, now() + Duration::seconds(900), &mut || 0)
            .unwrap();
        assert!(matches!(
            m.status.state,
            RemoteState::Stopped {
                reason: StopReason::Expired
            }
        ));
        let mut m = begin(true);
        approve(&mut m);
        let at = m.deadline.unwrap();
        m.step(Input::Deadline, at, &mut || 0).unwrap();
        assert!(m.secrets.is_none());
        assert!(matches!(
            m.status.state,
            RemoteState::Stopped {
                reason: StopReason::Expired
            }
        ));
        let mut m = begin(false);
        approve(&mut m);
        m.step(
            Input::Failure(Failure {
                retry: upgrade_action(401, None),
                ..Failure::network()
            }),
            now(),
            &mut || 0,
        )
        .unwrap();
        assert!(m.secrets.is_none());
        assert!(matches!(
            m.status.state,
            RemoteState::Stopped {
                reason: StopReason::Expired
            }
        ));
    }
    #[test]
    fn all_close_and_upgrade_refusals() {
        for code in [1000, 1008, 1011, 4444, 1006] {
            assert_eq!(close_action(code), Retry::Backoff);
        }
        assert_eq!(close_action(1012), Retry::Restart);
        assert_eq!(close_action(4001), Retry::Stop(StopReason::InUseElsewhere));
        assert_eq!(close_action(4003), Retry::Stop(StopReason::Removed));
        for status in [400, 403] {
            assert_eq!(
                upgrade_action(status, None),
                Retry::Stop(StopReason::UpdateRequired)
            );
        }
        assert_eq!(upgrade_action(401, None), Retry::Stop(StopReason::Removed));
        assert_eq!(upgrade_action(429, None), Retry::After(60));
        assert_eq!(upgrade_action(429, Some(42)), Retry::After(42));
        for status in [500, 503, 502] {
            assert_eq!(upgrade_action(status, None), Retry::Backoff);
        }
        for state in [EnrollmentState::Denied, EnrollmentState::Expired] {
            let mut m = begin(false);
            m.step(
                Input::Polled(Ok(PollResponse {
                    state,
                    ..Default::default()
                })),
                now(),
                &mut || 0,
            )
            .unwrap();
            assert!(matches!(m.status.state, RemoteState::Stopped { .. }));
        }
    }
}

#[cfg(test)]
mod review_tests {
    use super::*;
    #[test]
    fn provisional_stop_and_restore_keep_the_claim_deadline() {
        let mut m = super::tests::begin(false);
        super::tests::approve(&mut m);
        let expires = m.saved.as_ref().unwrap().claim_expires_at;
        m.step(
            Input::Failure(Failure {
                retry: Retry::Stop(StopReason::InUseElsewhere),
                ..Failure::network()
            }),
            super::tests::now(),
            &mut || 0,
        )
        .unwrap();
        assert_eq!(m.deadline, Some(expires));
        let (mut restored, _) =
            Machine::restore(m.saved.clone(), m.secrets.clone(), super::tests::now());
        assert_eq!(restored.deadline, Some(expires));
        let (expired, effects) = Machine::restore(m.saved.clone(), m.secrets.clone(), expires);
        assert!(matches!(
            expired.status.state,
            RemoteState::Stopped {
                reason: StopReason::Expired
            }
        ));
        assert!(effects.iter().any(|e| matches!(e, Effect::Delete(_))));
        for machine in [&mut m, &mut restored] {
            let (effects, deadline) = machine.step(Input::Deadline, expires, &mut || 0).unwrap();
            assert!(matches!(
                machine.status.state,
                RemoteState::Stopped {
                    reason: StopReason::Expired
                }
            ));
            assert!(machine.secrets.is_none());
            assert!(machine.saved.is_none());
            assert_eq!(deadline, None);
            assert!(effects.iter().any(|e| matches!(e, Effect::Delete(_))));
            assert!(!effects.iter().any(|e| matches!(e, Effect::SignOut { .. })));
        }
    }
    #[test]
    fn provisional_pause_and_restored_pause_keep_the_claim_deadline() {
        let mut m = super::tests::begin(false);
        super::tests::approve(&mut m);
        let expires = m.saved.as_ref().unwrap().claim_expires_at;
        super::tests::command(&mut m, Command::Pause(true), super::tests::now());
        assert_eq!(m.deadline, Some(expires));
        let (mut restored, _) =
            Machine::restore(m.saved.clone(), m.secrets.clone(), super::tests::now());
        assert!(matches!(restored.status.state, RemoteState::Paused));
        assert_eq!(restored.deadline, Some(expires));
        for machine in [&mut m, &mut restored] {
            let (effects, deadline) = machine.step(Input::Deadline, expires, &mut || 0).unwrap();
            assert!(matches!(
                machine.status.state,
                RemoteState::Stopped {
                    reason: StopReason::Expired
                }
            ));
            assert!(machine.secrets.is_none());
            assert!(machine.saved.is_none());
            assert_eq!(deadline, None);
            assert!(effects.iter().any(|e| matches!(e, Effect::Delete(_))));
            assert!(!effects.iter().any(|e| matches!(e, Effect::SignOut { .. })));
        }
    }
}
