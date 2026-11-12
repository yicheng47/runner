use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::time::Duration;

use runner_cli::client::{ClientError, SocketClient, ToolResponse};
use runner_core::protocol::{ClientEvent, EventError, EventSubscription};
use serde_json::{json, Value};

use crate::env::{BusContext, MissionEnv};
use crate::msg as bus;
use crate::{env, help, output, signal};
#[cfg(test)]
use clap::Parser;
#[cfg(test)]
use std::path::PathBuf;

mod ask;
mod chat;
mod cli;
mod crew;
mod daemon;
mod feed;
mod mission;
mod msg;
mod project;
mod role;
mod session;
mod shared;
mod status;

use ask::*;
use chat::*;
pub use cli::Cli;
use cli::*;
use crew::*;
use daemon::*;
use feed::*;
use mission::*;
use msg::*;
use project::*;
use role::*;
use session::*;
use shared::*;
use status::*;

pub fn run(cli: Cli) -> i32 {
    if let Command::Remote { command } = &cli.command {
        return crate::remote::run(command, cli.json);
    }
    if let Command::Hook { command } = &cli.command {
        return runner_cli::hook::run(command);
    }
    if let Command::Help { topic } = &cli.command {
        help::print(topic.as_deref());
        return 0;
    }
    let context = env::resolve();
    if let BusContext::Partial { missing } = &context {
        eprintln!(
            "runner: missing required env var(s): {}",
            missing.join(", ")
        );
        return 2;
    }
    if let Some(code) = run_local(&cli, &context) {
        return code;
    }
    let session = env::session_id();
    if let Err(error) = validate_remote(&cli, &context).and_then(|()| match &cli.command {
        Command::Mission { command } => require_chat(command, session.as_deref()),
        _ => Ok(()),
    }) {
        eprintln!("{}", error.message);
        return error.code;
    }
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("runner: failed to start runtime: {error}");
            return 1;
        }
    };
    let code = match runtime.block_on(run_remote(&cli, &context, session.as_deref())) {
        Ok(Some(response)) => {
            let response = postprocess_response(&cli, response);
            output::print(&response, cli.json, cli.quiet, output_view(&cli.command));
            0
        }
        Ok(None) => 0,
        Err(error) => {
            eprintln!("{}", error.message);
            error.code
        }
    };
    finish_runtime(runtime);
    code
}

fn finish_runtime(runtime: tokio::runtime::Runtime) {
    // A cancelled feed call may still be waiting on the longer blocking request deadline.
    runtime.shutdown_background();
}

fn run_local(cli: &Cli, context: &BusContext) -> Option<i32> {
    let BusContext::Mission(mission) = context else {
        return None;
    };
    match &cli.command {
        Command::Msg {
            command:
                MsgCommand::Post {
                    text,
                    to,
                    mission: target,
                    from,
                },
        } if same_mission(target.as_deref(), mission) => {
            if from.is_some() {
                eprintln!("runner msg post: --as is only valid outside the caller's mission");
                Some(2)
            } else {
                Some(bus::post(mission, text, to))
            }
        }
        Command::Msg {
            command:
                MsgCommand::Read {
                    all,
                    since,
                    from,
                    mission: target,
                },
        } if same_mission(target.as_deref(), mission) => {
            Some(bus::read(mission, *all, since.as_deref(), from.as_deref()))
        }
        Command::Signal {
            r#type,
            payload,
            mission: target,
            from,
        } if same_mission(target.as_deref(), mission) => {
            if from.is_some() {
                eprintln!("runner signal: --as is only valid outside the caller's mission");
                Some(2)
            } else {
                Some(signal::run(mission, r#type, payload.as_deref()))
            }
        }
        Command::Ask(args) if same_mission(args.mission.as_deref(), mission) => {
            if args.from.is_some() {
                eprintln!("runner ask: --as is only valid outside the caller's mission");
                return Some(2);
            }
            match ask_signal(args) {
                Ok((kind, payload)) => Some(signal::append(mission, kind, payload)),
                Err(error) => {
                    eprintln!("{}", error.message);
                    Some(error.code)
                }
            }
        }
        _ => None,
    }
}

fn same_mission(target: Option<&str>, mission: &MissionEnv) -> bool {
    target.is_none_or(|target| mission.mission_id.starts_with(target))
}

fn validate_remote(cli: &Cli, context: &BusContext) -> Result<(), CliError> {
    match &cli.command {
        Command::Mission {
            command: MissionCommand::AddRole { .. },
        } if matches!(context, BusContext::Mission(_)) => Err(CliError::usage(
            "runner mission add-role is for the person; ask the person to add a role",
        )),
        Command::Mission {
            command: MissionCommand::RemoveRole { .. },
        } if matches!(context, BusContext::Mission(_)) => Err(CliError::usage(
            "runner mission remove-role is for the person; ask the person to remove a role",
        )),
        Command::Role {
            command: RoleCommand::Create { fields, .. },
        } => {
            let runtime = fields
                .runtime
                .as_deref()
                .ok_or_else(|| CliError::usage("runner role create: --runtime is required"))?;
            runtime_command(runtime)?;
            parse_env(&fields.env)?;
            validate_permission(fields.permission.as_deref())?;
            Ok(())
        }
        Command::Role {
            command: RoleCommand::Update { fields, .. },
        } => {
            if let Some(runtime) = fields.runtime.as_deref() {
                runtime_command(runtime)?;
            }
            parse_env(&fields.env)?;
            validate_permission(fields.permission.as_deref())?;
            Ok(())
        }
        Command::Crew {
            command: CrewCommand::Add { runtime, .. },
        } => {
            if let Some(runtime) = runtime.as_deref() {
                runtime_command(runtime)?;
            }
            Ok(())
        }
        Command::Crew {
            command: CrewCommand::Set { runtime, .. },
        } => {
            if let Some(runtime) = runtime.as_deref().filter(|runtime| !runtime.is_empty()) {
                runtime_command(runtime)?;
            }
            Ok(())
        }
        Command::Msg {
            command: MsgCommand::Post { mission, .. },
        } if mission.is_none() && matches!(context, BusContext::OffBus) => Err(CliError::usage(
            "runner: --mission is required outside a mission",
        )),
        Command::Signal {
            r#type,
            payload,
            mission,
            ..
        } => {
            if r#type == "ask_lead" {
                return Err(CliError::usage(runner_core::model::ASK_LEAD_REMOVED_HINT));
            }
            if runner_core::model::KnownSignalType::from_name(r#type).is_none() {
                return Err(CliError::usage(format!(
                    "runner signal: unknown type {:?}",
                    r#type
                )));
            }
            signal::parse_payload(payload.as_deref()).map_err(CliError::usage)?;
            if mission.is_none() && matches!(context, BusContext::OffBus) {
                return Err(CliError::usage(
                    "runner: --mission is required outside a mission",
                ));
            }
            Ok(())
        }
        Command::Msg {
            command: MsgCommand::Read { .. },
        } => Err(CliError::usage(
            "runner msg read is available only inside a mission; use runner mission feed <mission>",
        )),
        Command::Ask(args) => {
            ask_signal(args)?;
            if args.mission.is_none() && matches!(context, BusContext::OffBus) {
                return Err(CliError::usage(
                    "runner ask: --mission is required outside a mission",
                ));
            }
            if args.from.is_none() {
                return Err(CliError::usage(
                    "runner ask: --as <handle> is required outside the caller's mission",
                ));
            }
            Ok(())
        }
        Command::Call { arguments, .. } => {
            parse_json_object(arguments.as_deref().unwrap_or("{}"))?;
            Ok(())
        }
        Command::Chat {
            command: ChatCommand::Start { role, runtime, .. },
        } => {
            if role.is_none() == runtime.is_none() {
                return Err(CliError::usage(
                    "runner chat start requires exactly one role or --runtime",
                ));
            }
            if let Some(runtime) = runtime.as_deref() {
                runtime_command(runtime)?;
            }
            Ok(())
        }
        Command::Mission {
            command:
                MissionCommand::Feed {
                    types,
                    follow,
                    limit,
                    ..
                },
        } => {
            parse_feed_types(types.as_deref())?;
            if *follow && *limit == Some(0) {
                return Err(CliError::usage(
                    "runner mission feed: --follow requires a positive --limit",
                ));
            }
            Ok(())
        }
        Command::Mission {
            command: MissionCommand::Move {
                project, unfile, ..
            },
        } if project.is_none() == !*unfile => Err(CliError::usage(
            "runner mission move requires exactly one of --project or --unfile",
        )),
        Command::Mission { command }
            if mission_arg(command).is_some_and(|target| {
                target.is_none() && matches!(context, BusContext::OffBus)
            }) =>
        {
            Err(CliError::usage(
                "runner: a mission reference is required outside a mission",
            ))
        }
        _ => Ok(()),
    }
}

async fn run_remote(
    cli: &Cli,
    context: &BusContext,
    session: Option<&str>,
) -> Result<Option<ToolResponse>, CliError> {
    if let Command::Daemon { command } = &cli.command {
        return daemon_command(command).map(Some);
    }
    let client = SocketClient::connect().await?;
    if matches!(cli.command, Command::Status) {
        return status_response(&client, context).map(Some);
    }
    if let Command::Mission {
        command:
            MissionCommand::Feed {
                mission,
                follow: true,
                since,
                limit,
                oldest_first,
                types,
                from,
                all,
            },
    } = &cli.command
    {
        let mission_id = resolve_mission_arg(&client, mission.as_deref(), context).await?;
        let filter = FeedFilter::new(types.as_deref(), from.as_deref(), *all)?;
        let stdout = std::io::stdout();
        let mut stdout = stdout.lock();
        follow_feed(
            &client,
            &mission_id,
            *since,
            *limit,
            *oldest_first,
            &filter,
            cli.json,
            cli.quiet,
            &mut stdout,
            &mut std::io::stderr(),
        )
        .await?;
        return Ok(None);
    }
    run_connected(&client, cli, context, session)
        .await
        .map(Some)
}

/// `session` is the caller's `RUNNER_SESSION_ID`, when it runs in an agent session.
async fn run_connected(
    client: &impl Caller,
    cli: &Cli,
    context: &BusContext,
    session: Option<&str>,
) -> Result<ToolResponse, CliError> {
    match &cli.command {
        Command::Remote { .. } | Command::Daemon { .. } | Command::Hook { .. } => unreachable!(),
        Command::Status => unreachable!(),
        Command::Project { command } => run_project(client, command).await,
        Command::Role { command } => run_role(client, command).await,
        Command::Crew { command } => run_crew(client, command).await,
        Command::Mission { command } => run_mission(client, command, context, session).await,
        Command::Chat { command } => run_chat(client, command).await,
        Command::Session { command } => run_session(client, command).await,
        Command::Msg { command } => run_msg(client, command, context).await,
        Command::Signal {
            r#type,
            payload,
            mission,
            from,
        } => {
            let mission_id = resolve_scoped_mission(client, mission.as_deref(), context).await?;
            let payload = signal::parse_payload(payload.as_deref()).map_err(CliError::usage)?;
            call(
                client,
                "mission_signal",
                mission_signal_args(&mission_id, r#type, payload, from.as_deref()),
            )
            .await
        }
        Command::Ask(args) => run_ask(client, args, context).await,
        Command::Call { tool, arguments } => {
            call(
                client,
                tool,
                parse_json_object(arguments.as_deref().unwrap_or("{}"))?,
            )
            .await
        }
        Command::Help { .. } => unreachable!(),
    }
}

#[cfg(test)]
mod tests;
