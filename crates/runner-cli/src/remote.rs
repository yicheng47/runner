use clap::Subcommand;
use runner_cli::client::{ClientError, SocketClient};
use runner_core::{
    protocol::DaemonClient,
    remote::{RemoteLogout, RemoteState, RemoteStatus},
};
use std::io::{IsTerminal, Write};

#[derive(Debug, Subcommand)]
pub enum RemoteCommand {
    /// Connect this computer to your account (interactive terminal required).
    Login {
        #[arg(long)]
        server: Option<String>,
    },
    /// Disconnect this computer and delete its local secrets.
    Logout,
    /// Show remote access status.
    Status,
}
const INTERACTIVE: &str = "`runner remote login` needs you at an interactive terminal outside an agent session. Use Runner → Settings → Remote access, or run it in your own terminal.";
pub fn login_allowed(stdin_tty: bool, stdout_tty: bool, agent: bool) -> bool {
    stdin_tty && stdout_tty && !agent
}
pub fn run(command: &RemoteCommand, json: bool) -> i32 {
    if matches!(command, RemoteCommand::Login { .. })
        && !login_allowed(
            std::io::stdin().is_terminal(),
            std::io::stdout().is_terminal(),
            std::env::var_os("RUNNER_HANDLE").is_some(),
        )
    {
        eprintln!("{INTERACTIVE}");
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
    let mut events = connection.subscribe();
    let mut status = typed(client.clone(), |c| c.remote_status()).await?;
    match command {
        RemoteCommand::Status => {
            if json {
                println!(
                    "{}",
                    serde_json::to_string(&status).map_err(|_| ClientError::Protocol(
                        "Could not encode remote status".into()
                    ))?
                );
            } else {
                print_status(&status);
            }
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
fn print_status(s: &RemoteStatus) {
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
    if let Some(fp) = &s.fingerprint {
        println!("Fingerprint  {fp}");
    }
    println!("Service      {}", s.origin);
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
