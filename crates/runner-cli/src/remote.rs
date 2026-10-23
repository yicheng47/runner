use clap::Subcommand;
use runner_cli::client::{ClientError, SocketClient};
use runner_core::{
    protocol::DaemonClient,
    remote::{RemoteLogout, RemoteState, RemoteStatus, RemoteTrust, Secret},
};
use std::io::{IsTerminal, Write};

#[derive(Debug, Subcommand)]
pub enum RemoteCommand {
    /// List this account's computers and their pairing status.
    Computers,
    /// Pair with a computer by its id (interactive terminal required).
    Pair { computer: String },
    /// Forget this computer's saved key for another computer.
    Forget { computer: String },
    /// Show another computer's projects, tabs, missions and sessions.
    Overview { computer: String },
    /// Connect this computer to your account (interactive terminal required).
    Login {
        #[arg(long)]
        server: Option<String>,
    },
    /// Disconnect this computer and delete its enrollment credential.
    Logout,
    /// Show remote access status.
    Status,
    /// Set or change the 6-digit pairing PIN (interactive terminal required).
    Pin,
    /// List devices paired with this computer.
    Devices,
    /// Revoke a paired device by its fingerprint.
    Revoke { fingerprint: String },
    /// Disconnect, unpair every device and create a new identity (interactive terminal required).
    ResetIdentity,
}
fn interactive_command(command: &RemoteCommand) -> Option<&'static str> {
    match command {
        RemoteCommand::Login { .. } => Some("login"),
        RemoteCommand::Pin => Some("pin"),
        RemoteCommand::Pair { .. } => Some("pair"),
        RemoteCommand::ResetIdentity => Some("reset-identity"),
        _ => None,
    }
}
pub fn login_allowed(stdin_tty: bool, stdout_tty: bool, agent: bool) -> bool {
    stdin_tty && stdout_tty && !agent
}
pub fn run(command: &RemoteCommand, json: bool) -> i32 {
    if interactive_command(command).is_some()
        && !login_allowed(
            std::io::stdin().is_terminal(),
            std::io::stdout().is_terminal(),
            std::env::var_os("RUNNER_HANDLE").is_some(),
        )
    {
        eprintln!("`runner remote {}` needs you at an interactive terminal outside an agent session. Use Runner → Settings → Remote access, or run it in your own terminal.", interactive_command(command).unwrap_or_default());
        return 1;
    }
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(_) => {
            eprintln!("Could not start remote access client.");
            return 1;
        }
    };
    let result = rt.block_on(execute(command, json));
    rt.shutdown_background();
    match result {
        Ok(code) => code,
        Err(e) => {
            eprintln!("{e}");
            match e {
                ClientError::NotRunning => 3,
                ClientError::Blocked => 5,
                _ => 1,
            }
        }
    }
}
async fn typed<T: Send + 'static>(
    client: DaemonClient,
    f: impl FnOnce(DaemonClient) -> Result<T, runner_core::protocol::ClientError> + Send + 'static,
) -> Result<T, ClientError> {
    tokio::task::spawn_blocking(move || f(client))
        .await
        .map_err(|_| ClientError::Protocol("Remote request failed".into()))?
        .map_err(|e| ClientError::Refused(e.message))
}
async fn refresh_event(
    client: DaemonClient,
    event: runner_core::protocol::ClientEvent,
) -> Result<Option<RemoteStatus>, ClientError> {
    if event.name == "remote/status" {
        typed(client, |c| c.remote_status()).await.map(Some)
    } else {
        Ok(None)
    }
}
async fn execute(command: &RemoteCommand, json: bool) -> Result<i32, ClientError> {
    let connection = SocketClient::connect().await?;
    let client = connection.daemon();
    let events = connection.subscribe();
    execute_connected(command, json, client, events).await
}
async fn execute_connected(
    command: &RemoteCommand,
    json: bool,
    client: DaemonClient,
    mut events: Box<dyn runner_core::protocol::EventSubscription>,
) -> Result<i32, ClientError> {
    let mut status = typed(client.clone(), |c| c.remote_status()).await?;
    match command {
        RemoteCommand::Computers => {
            let computers = typed(client, |c| c.remote_computers()).await?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string(&computers)
                        .map_err(|_| ClientError::Protocol("Could not encode computers".into()))?
                );
            } else {
                for computer in computers {
                    println!(
                        "{}  {}{}  {}  {}",
                        computer.id,
                        computer.name,
                        if computer.is_self {
                            " (this computer)"
                        } else {
                            ""
                        },
                        if computer.online { "online" } else { "offline" },
                        if computer.paired {
                            "paired"
                        } else {
                            "not paired"
                        }
                    );
                    if !computer.online {
                        if let Some(at) = computer.last_seen_at {
                            println!("  Last seen  {at}");
                        }
                    }
                }
            }
            Ok(0)
        }
        RemoteCommand::Pair { computer } => {
            let pin = rpassword::prompt_password("Other computer's PIN (6 digits): ")
                .map_err(|_| ClientError::Protocol("Could not read PIN".into()))?;
            if pin.len() != 6 || !pin.bytes().all(|b| b.is_ascii_digit()) {
                return Err(ClientError::Refused(
                    "The PIN must be exactly six digits.".into(),
                ));
            }
            let computer = computer.clone();
            pair_computer(client, computer, Secret(pin)).await?;
            if json {
                println!("null");
            } else {
                println!("Computer paired.");
            }
            Ok(0)
        }
        RemoteCommand::Forget { computer } => {
            let computer = computer.clone();
            typed(client, move |c| c.remote_forget(computer)).await?;
            if json {
                println!("null");
            } else {
                println!("Computer forgotten.");
            }
            Ok(0)
        }
        RemoteCommand::Overview { computer } => {
            let computer = computer.clone();
            let overview = typed(client, move |c| c.remote_overview(computer)).await?;
            println!(
                "{}",
                if json {
                    serde_json::to_string(&overview)
                } else {
                    serde_json::to_string_pretty(&overview)
                }
                .map_err(|_| ClientError::Protocol("Could not encode remote overview".into()))?
            );
            Ok(0)
        }
        RemoteCommand::Status => {
            let trust = typed(client.clone(), |c| c.remote_trust()).await?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string(&status_with_trust(&status, &trust)).map_err(|_| {
                        ClientError::Protocol("Could not encode remote status".into())
                    })?
                );
            } else {
                print_status(&status, &trust);
            }
            Ok(0)
        }
        RemoteCommand::Pin => {
            println!(
                "Runner’s service never sees this PIN, so it cannot pair a device on its own."
            );
            let pin = read_pin()?;
            typed(client, move |c| c.remote_set_pin(pin)).await?;
            println!("PIN saved. Devices already paired stay paired.");
            Ok(0)
        }
        RemoteCommand::ResetIdentity => {
            print!("Disconnect this computer, unpair every device and clear the PIN? [y/N] ");
            std::io::stdout()
                .flush()
                .map_err(|_| ClientError::Protocol("Could not display confirmation".into()))?;
            let mut line = String::new();
            std::io::stdin()
                .read_line(&mut line)
                .map_err(|_| ClientError::Protocol("Could not read confirmation".into()))?;
            if !matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
                println!("Identity reset cancelled.");
                return Ok(0);
            }
            let trust = typed(client.clone(), |c| c.remote_reset_identity()).await?;
            println!("Identity reset. Set a new PIN before pairing devices.");
            if let Some(fingerprint) = trust.fingerprint {
                println!("Fingerprint  {fingerprint}");
            }
            if let Some(error) = typed(client, |c| c.remote_status()).await?.last_error {
                println!("{error}");
            }
            Ok(0)
        }
        RemoteCommand::Devices => {
            let trust = typed(client, |c| c.remote_trust()).await?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string(&trust.devices).map_err(|_| ClientError::Protocol(
                        "Could not encode paired devices".into()
                    ))?
                );
            } else if trust.devices.is_empty() {
                println!("No devices paired.");
            } else {
                for device in trust.devices {
                    println!(
                        "{}\n  Fingerprint  {}\n  Paired       {}\n  Last seen    {}\n  Connected    {}",
                        device.name, device.fingerprint, device.paired_at, device.last_seen_at, device.connected
                    );
                }
            }
            Ok(0)
        }
        RemoteCommand::Revoke { fingerprint } => {
            let fingerprint = fingerprint.clone();
            typed(client, move |c| c.remote_revoke_device(fingerprint)).await?;
            println!("Device revoked.");
            Ok(0)
        }
        RemoteCommand::Logout => {
            if matches!(status.state, RemoteState::Off) {
                println!("This computer is not connected.");
                return Ok(0);
            }
            let result = typed(client, |c| c.remote_logout()).await?;
            match result {
                RemoteLogout::Removed => println!(
                    "Removed this computer from @{}'s account.",
                    status
                        .account
                        .as_ref()
                        .map(|a| a.username.as_str())
                        .unwrap_or("unknown")
                ),
                RemoteLogout::StillListed { error } => {
                    println!("Disconnected this computer. {error}")
                }
            }
            Ok(0)
        }
        RemoteCommand::Login { server } => {
            if let Some(server) = server {
                runner_core::remote::canonical_origin(server)
                    .map_err(|e| ClientError::Refused(e.into()))?;
            }
            let server = server.clone();
            status = tokio::select! {
                result = typed(client.clone(), move |c| c.remote_enroll_start(server, true)) => result?,
                _ = tokio::signal::ctrl_c() => { eprintln!("Enrollment cancelled."); return Ok(1); }
            };
            println!("Connect this computer to {}\nRemote access is a private beta, available only in mainland China for now.\nSigning in needs an account created from an invite code.\n", status.origin);
            if let RemoteState::Enrolling {
                verification_uri_complete,
                user_code,
                ..
            } = &status.state
            {
                println!("  Open         {verification_uri_complete}\n  Code         {user_code}\n  Fingerprint  {}\n\nCheck that the page shows the same fingerprint, then approve it there.", status.fingerprint.as_deref().unwrap_or("unknown"));
            }
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(1));
            let mut last_error = None;
            loop {
                match &status.state {
                    RemoteState::Connected { .. } => {
                        println!(
                            "\nConnected as @{}. This computer is {:?} on your account.",
                            status
                                .account
                                .as_ref()
                                .map(|a| a.username.as_str())
                                .unwrap_or("unknown"),
                            status.host_name.as_deref().unwrap_or("unknown")
                        );
                        return Ok(0);
                    }
                    RemoteState::Stopped { reason } => {
                        eprintln!("\nRemote access stopped: {reason:?}.");
                        return Ok(1);
                    }
                    RemoteState::Off => {
                        eprintln!(
                            "\n{}",
                            status
                                .last_error
                                .as_deref()
                                .unwrap_or("Enrollment cancelled or refused.")
                        );
                        return Ok(1);
                    }
                    RemoteState::Confirming {
                        account,
                        expires_at,
                    } => {
                        print!("\nApproved by @{} ({}).\nAllow remote access to this computer from @{} ({})? [y/N] ", account.username, account.display_name, account.username, account.display_name);
                        let _ = std::io::stdout().flush();
                        let line = tokio::task::spawn_blocking(|| {
                            let mut line = String::new();
                            std::io::stdin().read_line(&mut line).map(|_| line)
                        });
                        let timeout = expires_at
                            .signed_duration_since(chrono::Utc::now())
                            .to_std()
                            .unwrap_or_default();
                        let allow = tokio::select! {
                            line = line => line.ok().and_then(Result::ok).is_some_and(|s| matches!(s.trim().to_ascii_lowercase().as_str(), "y" | "yes")),
                            _ = tokio::signal::ctrl_c() => false,
                            _ = tokio::time::sleep(timeout) => { eprintln!("\nThe claim expired."); return Ok(1); }
                        };
                        status =
                            typed(client.clone(), move |c| c.remote_enroll_confirm(allow)).await?;
                        if !allow {
                            println!("Remote access refused.");
                            return Ok(1);
                        }
                        continue;
                    }
                    _ => {}
                }
                if status.last_error != last_error {
                    if let Some(e) = &status.last_error {
                        eprintln!("\n{e}");
                    }
                    last_error = status.last_error.clone();
                }
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {
                        if matches!(status.state, RemoteState::Enrolling { .. }) { let _ = typed(client.clone(), |c| c.remote_enroll_cancel()).await; }
                        else { let _ = typed(client.clone(), |c| c.remote_logout()).await; }
                        eprintln!("\nEnrollment cancelled."); return Ok(1);
                    }
                    _ = tick.tick() => {
                        if let RemoteState::Enrolling { expires_at, .. } = &status.state { let secs = expires_at.signed_duration_since(chrono::Utc::now()).num_seconds().max(0); print!("\rWaiting for approval… {}:{:02} left  ", secs / 60, secs % 60); let _ = std::io::stdout().flush(); }
                    }
                    event = events.recv() => {
                        match event {
                            Ok(event) => { if let Some(snapshot) = refresh_event(client.clone(), event).await? { status = snapshot; } },
                            Err(runner_core::protocol::EventError::Lagged(_)) => status = typed(client.clone(), |c| c.remote_status()).await?,
                            Err(_) => return Err(ClientError::NotRunning),
                        }
                    }
                }
            }
        }
    }
}
async fn pair_computer(
    client: DaemonClient,
    computer: String,
    pin: Secret,
) -> Result<(), ClientError> {
    typed(client, move |c| c.remote_pair(computer, pin)).await
}
fn status_with_trust(status: &RemoteStatus, trust: &RemoteTrust) -> serde_json::Value {
    let mut value = serde_json::to_value(status).expect("RemoteStatus serializes to JSON");
    value["trust"] = serde_json::to_value(trust).expect("RemoteTrust serializes to JSON");
    value
}
fn read_pin() -> Result<Secret, ClientError> {
    let pin = rpassword::prompt_password("New PIN (6 digits): ")
        .map_err(|_| ClientError::Protocol("Could not read PIN".into()))?;
    let confirmation = rpassword::prompt_password("Repeat PIN: ")
        .map_err(|_| ClientError::Protocol("Could not read PIN confirmation".into()))?;
    confirmed_pin(pin, confirmation)
}
fn confirmed_pin(pin: String, confirmation: String) -> Result<Secret, ClientError> {
    if pin != confirmation {
        return Err(ClientError::Refused("The PINs do not match.".into()));
    }
    if pin.len() != 6 || !pin.bytes().all(|c| c.is_ascii_digit()) {
        return Err(ClientError::Refused(
            "The PIN must be exactly six digits.".into(),
        ));
    }
    Ok(Secret(pin))
}
fn print_status(s: &RemoteStatus, trust: &RemoteTrust) {
    let state = match &s.state {
        RemoteState::Off => "not connected".into(),
        RemoteState::Enrolling { expires_at, .. } => {
            format!("waiting for approval; expires {expires_at}")
        }
        RemoteState::Confirming { expires_at, .. } => {
            format!("waiting for account confirmation; expires {expires_at}")
        }
        RemoteState::Connecting { attempt } => format!("connecting (attempt {attempt})"),
        RemoteState::Connected { since } => format!(
            "connected since {} ({} min)",
            since.format("%H:%M"),
            chrono::Utc::now()
                .signed_duration_since(*since)
                .num_minutes()
        ),
        RemoteState::Reconnecting {
            attempt,
            next_retry_at,
        } => format!("reconnecting (attempt {attempt}); next try {next_retry_at}"),
        RemoteState::Paused => "paused".into(),
        RemoteState::Stopped { reason } => format!("stopped: {reason:?}"),
    };
    println!("State        {state}");
    if let Some(a) = &s.account {
        println!("Account      @{} ({})", a.username, a.display_name);
    }
    if let Some(name) = &s.host_name {
        println!(
            "Computer     {name} (host {})",
            s.host_id.as_deref().unwrap_or("unknown")
        );
    }
    if let Some(fp) = trust.fingerprint.as_ref().or(s.fingerprint.as_ref()) {
        println!("Fingerprint  {fp}");
    }
    println!("Service      {}", s.origin);
    println!(
        "PIN          {}",
        if trust.pin_locked {
            "locked — unlock in Settings"
        } else if trust.pin_set {
            "set"
        } else {
            "not set"
        }
    );
    println!("Devices      {}", trust.devices.len());
    if let Some(e) = &s.last_error {
        println!("Last error   {e}");
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn connect_again_ignores_queued_off_and_older_enrollment_snapshots() {
        use runner_core::{
            protocol::{ClientEvent, EventSubscription, Request, Response, Transport},
            remote::{RemoteAccount, StopReason},
        };
        struct Source(std::sync::Mutex<RemoteStatus>);
        impl Transport for Source {
            fn call(
                &self,
                request: Request,
            ) -> Result<Response, runner_core::protocol::ClientError> {
                let mut status = self.0.lock().unwrap();
                match request {
                    Request::remote_enroll_start { .. } => {
                        status.state = RemoteState::Enrolling {
                            user_code: "BCDF-GHJK".into(),
                            verification_uri: "https://runner.example/hosts/add".into(),
                            verification_uri_complete:
                                "https://runner.example/hosts/add?code=BCDF-GHJK".into(),
                            expires_at: chrono::Utc::now() + chrono::Duration::minutes(10),
                        };
                        Ok(Response::remote_enroll_start(Ok(status.clone())))
                    }
                    Request::remote_status {} => Ok(Response::remote_status(Ok(status.clone()))),
                    Request::remote_enroll_confirm { allow: true } => {
                        status.state = RemoteState::Connected {
                            since: chrono::Utc::now(),
                        };
                        Ok(Response::remote_enroll_confirm(Ok(status.clone())))
                    }
                    _ => panic!("unexpected remote test request"),
                }
            }
            fn subscribe(&self) -> Box<dyn EventSubscription> {
                panic!("the test injects the queued event snapshots directly")
            }
        }
        for reason in [StopReason::Removed, StopReason::InUseElsewhere] {
            let source = std::sync::Arc::new(Source(std::sync::Mutex::new(RemoteStatus {
                state: RemoteState::Stopped { reason },
                host_id: Some("7383920468475904".into()),
                account: Some(RemoteAccount {
                    username: "jason".into(),
                    display_name: "Jason".into(),
                }),
                ..Default::default()
            })));
            let client = DaemonClient::new(source.clone());
            let enrolling = client.remote_enroll_start(None, true).unwrap();
            let old_off = ClientEvent {
                name: "remote/status".into(),
                payload: serde_json::to_value(RemoteStatus::default()).unwrap(),
            };
            let current = refresh_event(client.clone(), old_off.clone())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(current.state, enrolling.state);
            source.0.lock().unwrap().state = RemoteState::Confirming {
                account: RemoteAccount {
                    username: "jason".into(),
                    display_name: "Jason".into(),
                },
                expires_at: chrono::Utc::now() + chrono::Duration::minutes(5),
            };
            let current = refresh_event(
                client.clone(),
                ClientEvent {
                    name: "remote/status".into(),
                    payload: serde_json::to_value(enrolling).unwrap(),
                },
            )
            .await
            .unwrap()
            .unwrap();
            assert!(matches!(current.state, RemoteState::Confirming { .. }));
            client.remote_enroll_confirm(true).unwrap();
            let current = refresh_event(client, old_off).await.unwrap().unwrap();
            assert!(matches!(current.state, RemoteState::Connected { .. }));
        }
    }
    #[test]
    fn pin_confirmation_validates_without_echoing_the_input() {
        assert_eq!(
            confirmed_pin("001234".into(), "001234".into()).unwrap().0,
            "001234"
        );
        for (pin, repeat) in [
            ("123456", "654321"),
            ("12345", "12345"),
            ("1234567", "1234567"),
            ("0123456789", "0123456789"),
            ("１２３４５６", "１２３４５６"),
            ("12345x", "12345x"),
        ] {
            let error = confirmed_pin(pin.into(), repeat.into()).unwrap_err();
            assert!(!error.to_string().contains(pin));
            if pin == repeat {
                assert_eq!(error.to_string(), "The PIN must be exactly six digits.");
            }
        }
        for command in [RemoteCommand::Pin, RemoteCommand::ResetIdentity] {
            assert!(interactive_command(&command).is_some());
        }
        for command in [
            RemoteCommand::Devices,
            RemoteCommand::Revoke {
                fingerprint: "device".into(),
            },
        ] {
            assert!(interactive_command(&command).is_none());
        }
        let value = status_with_trust(&RemoteStatus::default(), &RemoteTrust::default());
        assert_eq!(value["state"]["kind"], "off");
        assert_eq!(value["trust"]["pin_set"], false);
    }
    #[test]
    fn only_person_at_two_terminals_can_login() {
        assert!(login_allowed(true, true, false));
        for input in [
            (false, true, false),
            (true, false, false),
            (true, true, true),
        ] {
            assert!(!login_allowed(input.0, input.1, input.2));
        }
    }
}

#[cfg(test)]
mod computer_tests {
    use super::*;
    use runner_core::protocol::{
        ClientError as ProtocolError, EventSubscription, Request, Response, Transport,
    };
    use std::sync::{Arc, Mutex};
    struct Source(Mutex<Vec<Request>>);
    impl Transport for Source {
        fn call(&self, request: Request) -> Result<Response, ProtocolError> {
            let response = match &request {
                Request::remote_status {} => Response::remote_status(Ok(RemoteStatus::default())),
                Request::remote_computers {} => Response::remote_computers(Ok(Vec::new())),
                Request::remote_forget { computer } => {
                    assert_eq!(computer, "42");
                    Response::remote_forget(Ok(()))
                }
                Request::remote_overview { computer } => {
                    assert_eq!(computer, "42");
                    Response::remote_overview(Ok(runner_core::remote::RemoteOverview(
                        runner_core::companion::Overview {
                            projects: vec![],
                            tabs: vec![],
                            missions: vec![],
                            sessions: Default::default(),
                        },
                    )))
                }
                Request::remote_pair { computer, pin } => {
                    assert_eq!(computer, "42");
                    assert_eq!(pin.0, "012345");
                    Response::remote_pair(Ok(()))
                }
                _ => panic!("unexpected computer command"),
            };
            self.0.lock().unwrap().push(request);
            Ok(response)
        }
        fn subscribe(&self) -> Box<dyn EventSubscription> {
            Box::new(NoEvents)
        }
    }
    struct NoEvents;
    impl EventSubscription for NoEvents {
        fn recv(
            &mut self,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = Result<
                            runner_core::protocol::ClientEvent,
                            runner_core::protocol::EventError,
                        >,
                    > + Send
                    + '_,
            >,
        > {
            Box::pin(std::future::pending())
        }
    }
    #[tokio::test]
    async fn computer_commands_make_only_the_expected_daemon_calls() {
        let source = Arc::new(Source(Mutex::new(Vec::new())));
        let client = DaemonClient::new(source.clone());
        for command in [
            RemoteCommand::Computers,
            RemoteCommand::Forget {
                computer: "42".into(),
            },
            RemoteCommand::Overview {
                computer: "42".into(),
            },
        ] {
            assert_eq!(
                execute_connected(&command, true, client.clone(), client.subscribe())
                    .await
                    .unwrap(),
                0
            );
        }
        pair_computer(client, "42".into(), Secret("012345".into()))
            .await
            .unwrap();
        let calls = source.0.lock().unwrap();
        assert_eq!(calls.len(), 7);
        assert!(matches!(calls[1], Request::remote_computers {}));
        assert!(matches!(calls[3], Request::remote_forget { .. }));
        assert!(matches!(calls[5], Request::remote_overview { .. }));
        assert!(matches!(calls[6], Request::remote_pair { .. }));
        assert!(!format!("{:?}", calls[6]).contains("012345"));
    }
}
