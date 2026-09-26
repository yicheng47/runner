use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use clap::{Args, Parser, Subcommand};
use runner_cli::client::{ClientError, SocketClient, ToolResponse};
use serde_json::{json, Value};

use crate::env::{BusContext, MissionEnv};
use crate::{env, help, msg, output, signal};

const DEFAULT_FEED_POLL_INTERVAL: Duration = Duration::from_secs(3);
#[cfg(not(test))]
const FEED_POLL_INTERVAL: Duration = DEFAULT_FEED_POLL_INTERVAL;
#[cfg(test)]
const FEED_POLL_INTERVAL: Duration = Duration::from_millis(1);
const DEFAULT_FEED_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
#[cfg(not(test))]
const FEED_REQUEST_TIMEOUT: Duration = DEFAULT_FEED_REQUEST_TIMEOUT;
#[cfg(test)]
const FEED_REQUEST_TIMEOUT: Duration = Duration::from_millis(50);

#[derive(Parser, Debug)]
#[command(
    name = "runner",
    bin_name = "runner",
    about = "Operate Runner missions, crews, roles, projects, and chats.",
    version,
    disable_help_subcommand = true
)]
pub struct Cli {
    /// Print the tool result as exact JSON.
    #[arg(long, global = true, conflicts_with = "quiet")]
    json: bool,
    /// Print only result IDs.
    #[arg(short = 'q', global = true, conflicts_with = "json")]
    quiet: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Check whether Runner is running.
    Status,
    /// Manage projects.
    Project {
        #[command(subcommand)]
        command: ProjectCommand,
    },
    /// Manage reusable agent roles.
    Role {
        #[command(subcommand)]
        command: RoleCommand,
    },
    /// Manage crews and their slots.
    Crew {
        #[command(subcommand)]
        command: CrewCommand,
    },
    /// Manage crew missions.
    Mission {
        #[command(subcommand)]
        command: MissionCommand,
    },
    /// Start direct chats.
    Chat {
        #[command(subcommand)]
        command: ChatCommand,
    },
    /// Inspect and manage sessions.
    Session {
        #[command(subcommand)]
        command: SessionCommand,
    },
    /// Post to or read a mission log.
    Msg {
        #[command(subcommand)]
        command: MsgCommand,
    },
    /// Emit a typed signal.
    Signal {
        /// Signal type from the Runner coordination protocol.
        r#type: String,
        /// JSON object carried by the signal.
        #[arg(long)]
        payload: Option<String>,
        /// Mission ID or unique prefix; required outside a mission.
        #[arg(long)]
        mission: Option<String>,
        /// Attribute the signal to this roster handle.
        #[arg(long = "as")]
        from: Option<String>,
    },
    /// Ask the lead, or ask the person with --human.
    Ask(AskArgs),
    /// Call any registered socket tool.
    Call {
        tool: String,
        arguments: Option<String>,
    },
    /// Print the command reference.
    Help { topic: Option<String> },
}

#[derive(Subcommand, Debug)]
enum ProjectCommand {
    /// List projects.
    List,
    /// Show one project by ID or exact name.
    Show { project: String },
    /// Create a project.
    Create {
        name: String,
        /// Project directory; defaults to the current directory.
        #[arg(long)]
        path: Option<PathBuf>,
    },
    /// Rename a project.
    Rename { project: String, name: String },
    /// Delete a project and archive its members.
    Delete {
        project: String,
        /// Allow deletion when the project has running members.
        #[arg(long)]
        force: bool,
    },
}

#[derive(Args, Debug, Default)]
struct RoleFields {
    /// Display name.
    #[arg(long)]
    name: Option<String>,
    /// Agent runtime registry name.
    #[arg(long)]
    runtime: Option<String>,
    /// Model override; pass an empty value to clear it.
    #[arg(long)]
    model: Option<String>,
    /// Reasoning effort; pass an empty value to clear it.
    #[arg(long)]
    effort: Option<String>,
    /// Permission mode: default, accept_edits, auto, or bypass.
    #[arg(long)]
    permission: Option<String>,
    /// Inline system prompt; pass an empty value to clear it.
    #[arg(long, conflicts_with = "prompt_file")]
    prompt: Option<String>,
    /// Read the system prompt from a file, or - for stdin.
    #[arg(long, conflicts_with = "prompt")]
    prompt_file: Option<PathBuf>,
    /// Append one runtime argument; repeat for multiple arguments.
    #[arg(long = "arg")]
    args: Vec<String>,
    /// Set one KEY=VALUE environment entry; repeat as needed.
    #[arg(long = "env")]
    env: Vec<String>,
    /// Working directory; pass an empty value to clear it.
    #[arg(long)]
    cwd: Option<String>,
}

#[derive(Subcommand, Debug)]
enum RoleCommand {
    /// List reusable roles.
    List,
    /// Show a role by handle.
    Show { handle: String },
    /// Create a role.
    Create {
        handle: String,
        #[command(flatten)]
        fields: RoleFields,
    },
    /// Update a role.
    Update {
        handle: String,
        #[command(flatten)]
        fields: RoleFields,
    },
    /// Delete a role.
    Delete { handle: String },
}

#[derive(Args, Debug, Default)]
struct CrewCreateFields {
    /// Short description of the crew.
    #[arg(long)]
    purpose: Option<String>,
    /// Default mission goal.
    #[arg(long)]
    goal: Option<String>,
    /// Read crew conventions from a file, or - for stdin.
    #[arg(long)]
    conventions_file: Option<PathBuf>,
}

#[derive(Args, Debug, Default)]
struct CrewFields {
    /// New crew name.
    #[arg(long)]
    name: Option<String>,
    /// Short description; pass an empty value to clear it.
    #[arg(long)]
    purpose: Option<String>,
    /// Default mission goal; pass an empty value to clear it.
    #[arg(long)]
    goal: Option<String>,
    /// Read crew conventions from a file, or - for stdin.
    #[arg(long)]
    conventions_file: Option<PathBuf>,
}

#[derive(Subcommand, Debug)]
enum CrewCommand {
    /// List crews.
    List,
    /// Show one crew and its slots.
    Show { crew: String },
    /// Create a crew.
    Create {
        name: String,
        #[command(flatten)]
        fields: CrewCreateFields,
    },
    /// Update a crew.
    Update {
        crew: String,
        #[command(flatten)]
        fields: CrewFields,
    },
    /// Delete a crew.
    Delete { crew: String },
    /// Add a role to a crew.
    Add {
        crew: String,
        role: String,
        /// Assign this in-crew handle.
        #[arg(long = "as")]
        handle: Option<String>,
        /// Override the role runtime for this slot.
        #[arg(long)]
        runtime: Option<String>,
        /// Override the role model for this slot.
        #[arg(long)]
        model: Option<String>,
        /// Override the role effort for this slot.
        #[arg(long)]
        effort: Option<String>,
    },
    /// Update one crew slot by handle.
    Set {
        crew: String,
        handle: String,
        /// Rename the in-crew handle.
        #[arg(long = "as")]
        new_handle: Option<String>,
        /// Override the runtime; pass an empty value to inherit.
        #[arg(long)]
        runtime: Option<String>,
        /// Override the model; pass an empty value to inherit.
        #[arg(long)]
        model: Option<String>,
        /// Override the effort; pass an empty value to inherit.
        #[arg(long)]
        effort: Option<String>,
    },
    /// Remove a slot from a crew.
    Remove { crew: String, handle: String },
    /// Make a slot the crew lead.
    Lead { crew: String, handle: String },
    /// Reorder every crew slot by handle.
    Order {
        crew: String,
        #[arg(required = true, num_args = 1..)]
        handles: Vec<String>,
    },
}

#[derive(Subcommand, Debug)]
enum MissionCommand {
    /// List active missions.
    List {
        /// Filter by crew ID or exact name.
        #[arg(long)]
        crew: Option<String>,
    },
    /// Show an operational mission snapshot.
    Show { mission: Option<String> },
    /// Start a mission.
    Start {
        /// Crew ID or exact name.
        #[arg(long)]
        crew: String,
        /// Mission goal text.
        #[arg(long, conflicts_with = "goal_file")]
        goal: Option<String>,
        /// Read the mission goal from a file, or - for stdin.
        #[arg(long, conflicts_with = "goal")]
        goal_file: Option<PathBuf>,
        /// Mission title; defaults from the goal or crew name.
        #[arg(long)]
        title: Option<String>,
        /// Project ID or exact name whose directory the mission uses.
        #[arg(long, conflicts_with = "cwd")]
        project: Option<String>,
        /// Mission directory; defaults to the current directory.
        #[arg(long, conflicts_with = "project")]
        cwd: Option<PathBuf>,
    },
    /// Stop a running mission.
    Stop { mission: Option<String> },
    /// Resume every stopped session in a mission.
    Resume { mission: Option<String> },
    /// Archive a mission.
    Archive { mission: Option<String> },
    /// Restore an archived mission to active lists.
    Unarchive { mission: Option<String> },
    /// Rename a mission.
    Rename { mission: String, title: String },
    /// Pin a mission.
    Pin { mission: Option<String> },
    /// Unpin a mission.
    Unpin { mission: Option<String> },
    /// Move a mission into a project or unfile it.
    Move {
        mission: Option<String>,
        /// Destination project ID or exact name.
        #[arg(long, conflicts_with = "unfile")]
        project: Option<String>,
        /// Remove the mission from its project.
        #[arg(long, conflicts_with = "project")]
        unfile: bool,
    },
    /// Print a window from the mission event feed.
    Feed {
        mission: Option<String>,
        /// Poll every 3 seconds until the mission ends or all its sessions exit.
        #[arg(long)]
        follow: bool,
        /// Start after this byte offset.
        #[arg(long)]
        since: Option<u64>,
        /// Maximum events to return.
        #[arg(long)]
        limit: Option<usize>,
        /// Ask the backend for chronological order.
        #[arg(long)]
        oldest_first: bool,
        /// Comma-separated signal kinds (or message) to include.
        #[arg(long, conflicts_with = "all")]
        types: Option<String>,
        /// Include only events from this handle.
        #[arg(long)]
        from: Option<String>,
        /// Include session_status and inbox_read noise.
        #[arg(long)]
        all: bool,
    },
    /// Answer a pending person question.
    Answer {
        mission: String,
        question_id: String,
        choice: String,
    },
}

#[derive(Subcommand, Debug)]
enum ChatCommand {
    /// Start a role-backed or runtime-only direct chat.
    Start {
        role: Option<String>,
        /// Start a role-free chat with this runtime.
        #[arg(long, conflicts_with = "role")]
        runtime: Option<String>,
        /// Model override.
        #[arg(long)]
        model: Option<String>,
        /// Reasoning effort override.
        #[arg(long)]
        effort: Option<String>,
        /// Project ID or exact name whose directory the chat uses.
        #[arg(long, conflicts_with = "cwd")]
        project: Option<String>,
        /// Chat directory; defaults to the current directory.
        #[arg(long, conflicts_with = "project")]
        cwd: Option<PathBuf>,
    },
}

#[derive(Subcommand, Debug)]
enum SessionCommand {
    /// List recent direct-chat sessions.
    List,
    /// Show one session with its live agent status.
    Show { session: String },
    /// Stop a session and leave it resumable.
    Stop { session: String },
    /// Stop and archive a direct chat.
    Archive { session: String },
    /// Resume a stopped session.
    Resume { session: String },
    /// Restart a mission session with a fresh conversation.
    Restart { session: String },
}

#[derive(Subcommand, Debug)]
enum MsgCommand {
    /// Post a mission message.
    Post {
        text: String,
        /// Direct the message to one roster handle.
        #[arg(long)]
        to: Option<String>,
        /// Mission ID or unique prefix; required outside a mission.
        #[arg(long)]
        mission: Option<String>,
        /// Attribute the message to this roster handle.
        #[arg(long = "as")]
        from: Option<String>,
    },
    /// Read the current mission inbox.
    Read {
        /// Return messages after this event ID.
        #[arg(long)]
        since: Option<String>,
        /// Filter by sender handle.
        #[arg(long)]
        from: Option<String>,
        /// Mission reference; only the caller's mission is supported.
        #[arg(long)]
        mission: Option<String>,
    },
}

#[derive(Args, Debug)]
struct AskArgs {
    /// Question for the mission lead.
    question: Option<String>,
    /// Additional context for a lead question.
    #[arg(long)]
    context: Option<String>,
    /// Ask the person at the app instead of the lead.
    #[arg(long, conflicts_with = "question")]
    human: Option<String>,
    /// Comma-separated choices for --human.
    #[arg(long)]
    choices: Option<String>,
    /// Mission ID or unique prefix; required outside a mission.
    #[arg(long)]
    mission: Option<String>,
    /// Attribute the question to this roster handle.
    #[arg(long = "as")]
    from: Option<String>,
}

#[derive(Debug)]
struct CliError {
    code: i32,
    message: String,
}

impl CliError {
    fn usage(message: impl Into<String>) -> Self {
        Self {
            code: 2,
            message: message.into(),
        }
    }
}

impl From<ClientError> for CliError {
    fn from(error: ClientError) -> Self {
        let code = match error {
            ClientError::NotRunning => 3,
            ClientError::Blocked => 5,
            ClientError::Refused(_) | ClientError::Protocol(_) => 1,
        };
        Self {
            code,
            message: error.to_string(),
        }
    }
}

trait ToolCaller {
    async fn call(&self, name: &str, arguments: Value) -> Result<ToolResponse, CliError>;
}

impl ToolCaller for SocketClient {
    async fn call(&self, name: &str, arguments: Value) -> Result<ToolResponse, CliError> {
        SocketClient::call(self, name, arguments)
            .await
            .map_err(Into::into)
    }
}

pub fn run(cli: Cli) -> i32 {
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
    if let Err(error) = validate_remote(&cli, &context) {
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
    match runtime.block_on(run_remote(&cli, &context)) {
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
    }
}

fn output_view(command: &Command) -> output::View {
    use output::View;

    match command {
        Command::Status => View::Status,
        Command::Project { command } => match command {
            ProjectCommand::List => View::ProjectList,
            ProjectCommand::Show { .. }
            | ProjectCommand::Create { .. }
            | ProjectCommand::Rename { .. } => View::Project,
            ProjectCommand::Delete { .. } => View::Confirmation("Deleted project"),
        },
        Command::Role { command } => match command {
            RoleCommand::List => View::RoleList,
            RoleCommand::Show { .. } | RoleCommand::Create { .. } | RoleCommand::Update { .. } => {
                View::Role
            }
            RoleCommand::Delete { .. } => View::Confirmation("Deleted role"),
        },
        Command::Crew { command } => match command {
            CrewCommand::List => View::CrewList,
            CrewCommand::Show { .. } => View::CrewShow,
            CrewCommand::Create { .. } | CrewCommand::Update { .. } => View::Crew,
            CrewCommand::Delete { .. } => View::Confirmation("Deleted crew"),
            CrewCommand::Add { .. }
            | CrewCommand::Set { .. }
            | CrewCommand::Remove { .. }
            | CrewCommand::Lead { .. }
            | CrewCommand::Order { .. } => View::Confirmation("Updated crew slots"),
        },
        Command::Mission { command } => match command {
            MissionCommand::List { .. } => View::MissionList,
            MissionCommand::Show { .. } => View::MissionShow,
            MissionCommand::Feed { .. } => View::MissionFeed,
            MissionCommand::Resume { .. } => View::Mission,
            MissionCommand::Answer { .. } => View::Confirmation("Answered question"),
            MissionCommand::Start { .. }
            | MissionCommand::Stop { .. }
            | MissionCommand::Archive { .. }
            | MissionCommand::Unarchive { .. }
            | MissionCommand::Rename { .. }
            | MissionCommand::Pin { .. }
            | MissionCommand::Unpin { .. }
            | MissionCommand::Move { .. } => View::Mission,
        },
        Command::Chat { .. } => View::Confirmation("Started session"),
        Command::Session { command } => match command {
            SessionCommand::List => View::SessionList,
            SessionCommand::Show { .. } => View::SessionShow,
            SessionCommand::Stop { .. } => View::Confirmation("Stopped session"),
            SessionCommand::Archive { .. } => View::Confirmation("Archived session"),
            SessionCommand::Resume { .. } => View::Confirmation("Resumed session"),
            SessionCommand::Restart { .. } => View::Confirmation("Restarted session"),
        },
        Command::Msg { .. } => View::Confirmation("Posted message"),
        Command::Signal { .. } => View::Confirmation("Posted signal"),
        Command::Ask(_) => View::Confirmation("Posted question"),
        Command::Call { .. } => View::Generic,
        Command::Help { .. } => unreachable!(),
    }
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
                Some(msg::post(mission, text, to.as_deref()))
            }
        }
        Command::Msg {
            command:
                MsgCommand::Read {
                    since,
                    from,
                    mission: target,
                },
        } if same_mission(target.as_deref(), mission) => {
            Some(msg::read(mission, since.as_deref(), from.as_deref()))
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

fn mission_arg(command: &MissionCommand) -> Option<&Option<String>> {
    match command {
        MissionCommand::Show { mission }
        | MissionCommand::Stop { mission }
        | MissionCommand::Resume { mission }
        | MissionCommand::Archive { mission }
        | MissionCommand::Unarchive { mission }
        | MissionCommand::Pin { mission }
        | MissionCommand::Unpin { mission }
        | MissionCommand::Move { mission, .. }
        | MissionCommand::Feed { mission, .. } => Some(mission),
        _ => None,
    }
}

async fn run_remote(cli: &Cli, context: &BusContext) -> Result<Option<ToolResponse>, CliError> {
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
    run_connected(&client, cli, context).await.map(Some)
}

async fn run_connected(
    client: &impl ToolCaller,
    cli: &Cli,
    context: &BusContext,
) -> Result<ToolResponse, CliError> {
    match &cli.command {
        Command::Status => unreachable!(),
        Command::Project { command } => run_project(client, command).await,
        Command::Role { command } => run_role(client, command).await,
        Command::Crew { command } => run_crew(client, command).await,
        Command::Mission { command } => run_mission(client, command, context).await,
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

fn status_response(client: &SocketClient, context: &BusContext) -> Result<ToolResponse, CliError> {
    let debug = cfg!(debug_assertions);
    let app_data = runner_core::app_paths::app_data_dir(debug)
        .ok_or_else(|| CliError::usage("Runner app data directory could not be resolved"))?;
    let sidecar = app_data
        .join("bin")
        .join(format!("runner{}", std::env::consts::EXE_SUFFIX));
    let home = runner_core::app_paths::home_dir()
        .ok_or_else(|| CliError::usage("Runner home directory could not be resolved"))?;
    let skills = skill_statuses(&home, debug);
    let command = command_install_status(&home, &sidecar, &app_data, debug);
    let mode = match context {
        BusContext::Mission(mission) => {
            format!(
                "inside mission {} as @{}",
                mission.mission_id, mission.handle
            )
        }
        BusContext::OffBus => "outside a mission".to_owned(),
        BusContext::Partial { .. } => unreachable!(),
    };
    response(json!({
        "cli_version": env!("CARGO_PKG_VERSION"),
        "app_version": client.app_version(),
        "socket": client.endpoint().to_string(),
        "sidecar": sidecar,
        "sidecar_present": sidecar.is_file(),
        "mode": mode,
        "command": command,
        "skills": skills,
    }))
}

fn command_install_status(home: &Path, sidecar: &Path, app_data: &Path, debug: bool) -> Value {
    let process_path = std::env::var("PATH").unwrap_or_default();
    command_install_status_with_path(home, sidecar, app_data, debug, &process_path)
}

fn command_install_status_with_path(
    home: &Path,
    sidecar: &Path,
    app_data: &Path,
    debug: bool,
    process_path: &str,
) -> Value {
    #[cfg(windows)]
    let _ = home;
    #[cfg(windows)]
    let status = {
        let sidecar_dir = sidecar.parent().unwrap_or(Path::new(""));
        let user_path = read_windows_user_path().unwrap_or_default();
        let mut search_path = path_without_runner_entries(
            process_path,
            app_data,
            runner_core::command_install::PathStyle::Windows,
        );
        if !search_path.is_empty() {
            search_path.push(';');
        }
        search_path.push_str(&sidecar_dir.to_string_lossy());
        runner_core::command_install::inspect_windows_command(
            sidecar_dir,
            &user_path,
            &search_path,
            debug,
        )
    };
    #[cfg(not(windows))]
    let status = runner_core::command_install::inspect_unix_command(
        sidecar,
        &path_without_runner_entries(
            process_path,
            app_data,
            runner_core::command_install::PathStyle::Unix,
        ),
        &home.join(".local/bin"),
        Path::new("/usr/local/bin"),
        debug,
    );
    serde_json::to_value(status).expect("command status is serializable")
}

fn path_without_runner_entries(
    search_path: &str,
    app_data: &Path,
    style: runner_core::command_install::PathStyle,
) -> String {
    let separator = match style {
        runner_core::command_install::PathStyle::Unix => ":",
        runner_core::command_install::PathStyle::Windows => ";",
    };
    runner_core::command_install::path_entries(search_path, style)
        .into_iter()
        .filter(|entry| !runner_core::command_install::path_is_within(entry, app_data, style))
        .map(|entry| entry.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(separator)
}

#[cfg(windows)]
fn read_windows_user_path() -> Result<String, CliError> {
    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY_CURRENT_USER, KEY_QUERY_VALUE,
    };

    let environment = "Environment"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let name = "Path"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let mut key = std::ptr::null_mut();
    let status = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            environment.as_ptr(),
            0,
            KEY_QUERY_VALUE,
            &mut key,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(CliError {
            code: 1,
            message: format!("open HKCU\\Environment failed with code {status}"),
        });
    }
    let mut bytes = 0;
    let status = unsafe {
        RegQueryValueExW(
            key,
            name.as_ptr(),
            std::ptr::null(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut bytes,
        )
    };
    if status == ERROR_FILE_NOT_FOUND {
        unsafe { RegCloseKey(key) };
        return Ok(String::new());
    }
    if status != ERROR_SUCCESS {
        unsafe { RegCloseKey(key) };
        return Err(CliError {
            code: 1,
            message: format!("read HKCU\\Environment Path size failed with code {status}"),
        });
    }
    let mut data = vec![0u16; bytes as usize / 2];
    let status = unsafe {
        RegQueryValueExW(
            key,
            name.as_ptr(),
            std::ptr::null(),
            std::ptr::null_mut(),
            data.as_mut_ptr().cast(),
            &mut bytes,
        )
    };
    unsafe { RegCloseKey(key) };
    if status != ERROR_SUCCESS {
        return Err(CliError {
            code: 1,
            message: format!("read HKCU\\Environment Path failed with code {status}"),
        });
    }
    while data.last() == Some(&0) {
        data.pop();
    }
    Ok(String::from_utf16_lossy(&data))
}

fn skill_statuses(home: &Path, debug: bool) -> Vec<Value> {
    let skill_name = runner_core::runner_skill_name(debug);
    runner_core::RUNNER_SKILL_ROOTS
        .iter()
        .map(|relative| {
            let folder = home.join(relative).join(skill_name);
            let state = if !folder.exists() {
                "missing"
            } else if folder.join(runner_core::RUNNER_SKILL_MARKER).is_file() {
                "managed"
            } else {
                "foreign"
            };
            json!({"root": home.join(relative), "folder": folder, "state": state})
        })
        .collect()
}

async fn run_project(
    client: &impl ToolCaller,
    command: &ProjectCommand,
) -> Result<ToolResponse, CliError> {
    match command {
        ProjectCommand::List => call(client, "project_list", json!({})).await,
        ProjectCommand::Show { project } => {
            let project = resolve_named(client, "project", "project_list", project).await?;
            call(client, "project_get", json!({"id": project.id})).await
        }
        ProjectCommand::Create { name, path } => {
            let path = absolute_path(path.as_deref().unwrap_or(Path::new(".")))?;
            call(client, "project_create", json!({"name": name, "cwd": path})).await
        }
        ProjectCommand::Rename { project, name } => {
            let project = resolve_named(client, "project", "project_list", project).await?;
            call(
                client,
                "project_rename",
                json!({"id": project.id, "name": name}),
            )
            .await
        }
        ProjectCommand::Delete { project, force } => {
            let project = resolve_named(client, "project", "project_list", project).await?;
            let mut response = call(
                client,
                "project_delete",
                json!({"id": project.id, "force": force}),
            )
            .await?;
            response
                .value
                .as_object_mut()
                .unwrap()
                .insert("id".into(), json!(project.id));
            Ok(response)
        }
    }
}

async fn run_role(
    client: &impl ToolCaller,
    command: &RoleCommand,
) -> Result<ToolResponse, CliError> {
    match command {
        RoleCommand::List => call(client, "role_list", json!({})).await,
        RoleCommand::Show { handle } => {
            let role = resolve_role(client, handle).await?;
            call(client, "role_get_by_handle", json!({"handle": role.name})).await
        }
        RoleCommand::Create { handle, fields } => {
            let runtime = fields
                .runtime
                .as_deref()
                .ok_or_else(|| CliError::usage("runner role create: --runtime is required"))?;
            let mut args = role_fields(fields, false)?;
            let object = args.as_object_mut().unwrap();
            object.insert("handle".into(), json!(handle));
            object.insert(
                "display_name".into(),
                json!(fields.name.as_deref().unwrap_or(handle)),
            );
            object.insert("runtime".into(), json!(runtime));
            object.insert("command".into(), json!(runtime_command(runtime)?));
            call(client, "role_create", args).await
        }
        RoleCommand::Update { handle, fields } => {
            let role = resolve_role(client, handle).await?;
            call(
                client,
                "role_update",
                json!({"id": role.id, "input": role_fields(fields, true)?}),
            )
            .await
        }
        RoleCommand::Delete { handle } => {
            let role = resolve_role(client, handle).await?;
            call(client, "role_delete", json!({"id": role.id})).await
        }
    }
}

async fn run_crew(
    client: &impl ToolCaller,
    command: &CrewCommand,
) -> Result<ToolResponse, CliError> {
    match command {
        CrewCommand::List => call(client, "crew_list", json!({})).await,
        CrewCommand::Show { crew } => {
            let crew = resolve_named(client, "crew", "crew_list", crew).await?;
            let detail = call(client, "crew_get", json!({"id": crew.id})).await?;
            let slots = call(client, "slot_list", json!({"crew_id": crew.id})).await?;
            response(json!({"crew": detail.value, "slots": slots.value}))
        }
        CrewCommand::Create { name, fields } => {
            let mut args = crew_create_fields(fields)?;
            args.as_object_mut()
                .unwrap()
                .insert("name".into(), json!(name));
            call(client, "crew_create", args).await
        }
        CrewCommand::Update { crew, fields } => {
            let crew = resolve_named(client, "crew", "crew_list", crew).await?;
            call(
                client,
                "crew_update",
                json!({"id": crew.id, "input": crew_fields(fields)?}),
            )
            .await
        }
        CrewCommand::Delete { crew } => {
            let crew = resolve_named(client, "crew", "crew_list", crew).await?;
            call(client, "crew_delete", json!({"id": crew.id})).await
        }
        CrewCommand::Add {
            crew,
            role,
            handle,
            runtime,
            model,
            effort,
        } => {
            let crew = resolve_named(client, "crew", "crew_list", crew).await?;
            let role = resolve_role(client, role).await?;
            let mut args = json!({
                "crew_id": crew.id,
                "role_id": role.id,
                "slot_handle": handle.as_deref().unwrap_or(&role.name),
            });
            insert_clearable(&mut args, "runtime_override", runtime.as_deref(), false);
            insert_clearable(&mut args, "model_override", model.as_deref(), false);
            let created = call(client, "slot_create", args).await?;
            if let Some(effort) = effort {
                let slot_id = id_from(&created.value)?;
                call(
                    client,
                    "slot_update",
                    json!({
                        "slot_id": slot_id,
                        "input": {"effort_override": nullable(effort)},
                    }),
                )
                .await
            } else {
                Ok(created)
            }
        }
        CrewCommand::Set {
            crew,
            handle,
            new_handle,
            runtime,
            model,
            effort,
        } => {
            let crew = resolve_named(client, "crew", "crew_list", crew).await?;
            let slot = resolve_slot(client, &crew.id, handle).await?;
            let mut input = json!({});
            insert_string(&mut input, "slot_handle", new_handle.as_deref());
            insert_clearable(&mut input, "runtime_override", runtime.as_deref(), true);
            insert_clearable(&mut input, "model_override", model.as_deref(), true);
            insert_clearable(&mut input, "effort_override", effort.as_deref(), true);
            call(
                client,
                "slot_update",
                json!({"slot_id": slot.id, "input": input}),
            )
            .await
        }
        CrewCommand::Remove { crew, handle } => {
            let crew = resolve_named(client, "crew", "crew_list", crew).await?;
            let slot = resolve_slot(client, &crew.id, handle).await?;
            call(client, "slot_delete", json!({"slot_id": slot.id})).await
        }
        CrewCommand::Lead { crew, handle } => {
            let crew = resolve_named(client, "crew", "crew_list", crew).await?;
            let slot = resolve_slot(client, &crew.id, handle).await?;
            call(client, "slot_set_lead", json!({"slot_id": slot.id})).await
        }
        CrewCommand::Order { crew, handles } => {
            let crew = resolve_named(client, "crew", "crew_list", crew).await?;
            let slots = list(client, "slot_list", json!({"crew_id": crew.id})).await?;
            let mut ids = Vec::new();
            for handle in handles {
                let matches = slots
                    .iter()
                    .filter(|slot| field(slot, "slot_handle") == Some(handle.as_str()))
                    .collect::<Vec<_>>();
                if matches.len() != 1 {
                    return Err(CliError::usage(format!(
                        "crew slot handle not found: {handle}"
                    )));
                }
                ids.push(field(matches[0], "id").unwrap().to_owned());
            }
            if ids.len() != slots.len() {
                return Err(CliError::usage(
                    "runner crew order must name every slot exactly once",
                ));
            }
            if ids.iter().collect::<std::collections::BTreeSet<_>>().len() != ids.len() {
                return Err(CliError::usage(
                    "runner crew order must name every slot exactly once",
                ));
            }
            call(
                client,
                "slot_reorder",
                json!({"crew_id": crew.id, "ordered_slot_ids": ids}),
            )
            .await
        }
    }
}

async fn run_mission(
    client: &impl ToolCaller,
    command: &MissionCommand,
    context: &BusContext,
) -> Result<ToolResponse, CliError> {
    match command {
        MissionCommand::List { crew } => {
            let crew_id = match crew {
                Some(crew) => Some(resolve_named(client, "crew", "crew_list", crew).await?.id),
                None => None,
            };
            call(client, "mission_list_summary", json!({"crew_id": crew_id})).await
        }
        MissionCommand::Show { mission } => {
            let id = resolve_mission_arg(client, mission.as_deref(), context).await?;
            call(client, "mission_status", json!({"id": id})).await
        }
        MissionCommand::Start {
            crew,
            goal,
            goal_file,
            title,
            project,
            cwd,
        } => {
            let crew = resolve_named(client, "crew", "crew_list", crew).await?;
            let goal = read_text(goal.as_deref(), goal_file.as_deref(), "goal")?;
            let title = title.clone().unwrap_or_else(|| {
                goal.as_deref()
                    .and_then(|goal| goal.lines().find(|line| !line.trim().is_empty()))
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .map(str::to_owned)
                    .unwrap_or_else(|| {
                        format!("{} {}", crew.name, chrono::Local::now().format("%Y-%m-%d"))
                    })
            });
            let project_id = match project {
                Some(project) => Some(
                    resolve_named(client, "project", "project_list", project)
                        .await?
                        .id,
                ),
                None => None,
            };
            let cwd = if project_id.is_none() {
                Some(absolute_path(cwd.as_deref().unwrap_or(Path::new(".")))?)
            } else {
                None
            };
            call(
                client,
                "mission_start",
                json!({
                    "crew_id": crew.id,
                    "project_id": project_id,
                    "title": title,
                    "goal_override": goal,
                    "cwd": cwd,
                }),
            )
            .await
        }
        MissionCommand::Stop { mission } => {
            mission_lifecycle(client, "mission_stop", mission.as_deref(), context).await
        }
        MissionCommand::Resume { mission } => {
            mission_lifecycle(client, "mission_resume", mission.as_deref(), context).await
        }
        MissionCommand::Archive { mission } => {
            mission_lifecycle(client, "mission_archive", mission.as_deref(), context).await
        }
        MissionCommand::Unarchive { mission } => {
            mission_lifecycle(client, "mission_unarchive", mission.as_deref(), context).await
        }
        MissionCommand::Rename { mission, title } => {
            let id = resolve_mission(client, mission).await?;
            call(client, "mission_rename", json!({"id": id, "title": title})).await
        }
        MissionCommand::Pin { mission } | MissionCommand::Unpin { mission } => {
            let id = resolve_mission_arg(client, mission.as_deref(), context).await?;
            call(
                client,
                "mission_pin",
                json!({
                    "id": id,
                    "pinned": matches!(command, MissionCommand::Pin { .. }),
                }),
            )
            .await
        }
        MissionCommand::Move {
            mission,
            project,
            unfile: _,
        } => {
            let mission_id = resolve_mission_arg(client, mission.as_deref(), context).await?;
            let project_id = match project {
                Some(project) => Some(
                    resolve_named(client, "project", "project_list", project)
                        .await?
                        .id,
                ),
                None => None,
            };
            call(
                client,
                "mission_set_project",
                json!({"mission_id": mission_id, "project_id": project_id}),
            )
            .await
        }
        MissionCommand::Feed {
            mission,
            follow: _,
            since,
            limit,
            oldest_first,
            types: _,
            from: _,
            all: _,
        } => {
            let mission_id = resolve_mission_arg(client, mission.as_deref(), context).await?;
            call(
                client,
                "mission_feed",
                json!({
                    "mission_id": mission_id,
                    "since_offset": since,
                    "limit": limit,
                    "order": if *oldest_first { "oldest_first" } else { "newest_first" },
                }),
            )
            .await
        }
        MissionCommand::Answer {
            mission,
            question_id,
            choice,
        } => {
            let mission_id = resolve_mission(client, mission).await?;
            call(
                client,
                "mission_signal",
                mission_signal_args(
                    &mission_id,
                    "human_response",
                    json!({"question_id": question_id, "choice": choice}),
                    None,
                ),
            )
            .await
        }
    }
}

#[derive(Debug)]
struct FeedFilter {
    types: Option<HashSet<String>>,
    from: Option<String>,
    all: bool,
}

impl FeedFilter {
    fn new(types: Option<&str>, from: Option<&str>, all: bool) -> Result<Self, CliError> {
        Ok(Self {
            types: parse_feed_types(types)?,
            from: from.map(str::to_owned),
            all,
        })
    }

    fn is_explicit(&self) -> bool {
        self.types.is_some() || self.from.is_some() || self.all
    }

    fn matches(&self, entry: &Value) -> bool {
        let event = entry.get("event").unwrap_or(entry);
        if self
            .from
            .as_deref()
            .is_some_and(|from| field(event, "from") != Some(from))
        {
            return false;
        }
        let event_type = field(event, "type").or_else(|| field(event, "kind"));
        if let Some(types) = &self.types {
            return event_type.is_some_and(|kind| types.contains(kind));
        }
        self.all
            || event
                .pointer("/payload/status/lifecycle")
                .and_then(Value::as_str)
                == Some("error")
            || event
                .pointer("/payload/status/observation/outcome")
                .and_then(Value::as_str)
                == Some("failed")
            || !event_type.is_some_and(|kind| {
                matches!(kind, "session_status" | "runner_status" | "inbox_read")
            })
    }
}

fn parse_feed_types(types: Option<&str>) -> Result<Option<HashSet<String>>, CliError> {
    let Some(types) = types else {
        return Ok(None);
    };
    let values = types
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect::<HashSet<_>>();
    if values.is_empty() {
        return Err(CliError::usage(
            "runner mission feed: --types requires at least one event kind",
        ));
    }
    Ok(Some(values))
}

fn filter_feed_response(response: &ToolResponse, filter: &FeedFilter) -> ToolResponse {
    let mut value = response.value.clone();
    if let Some(events) = value.get_mut("events").and_then(Value::as_array_mut) {
        events.retain(|entry| filter.matches(entry));
    }
    ToolResponse {
        raw_json: serde_json::to_string(&value).unwrap_or_else(|_| response.raw_json.clone()),
        value,
    }
}

fn postprocess_response(cli: &Cli, response: ToolResponse) -> ToolResponse {
    let Command::Mission {
        command:
            MissionCommand::Feed {
                follow: false,
                types,
                from,
                all,
                ..
            },
    } = &cli.command
    else {
        return response;
    };
    let filter = FeedFilter::new(types.as_deref(), from.as_deref(), *all)
        .expect("feed filters were validated before connecting");
    if cli.json && !filter.is_explicit() {
        response
    } else {
        filter_feed_response(&response, &filter)
    }
}

#[allow(clippy::too_many_arguments)]
async fn follow_feed(
    client: &impl ToolCaller,
    mission_id: &str,
    since: Option<u64>,
    limit: Option<usize>,
    oldest_first: bool,
    filter: &FeedFilter,
    json: bool,
    quiet: bool,
    writer: &mut impl std::io::Write,
    diagnostics: &mut impl std::io::Write,
) -> Result<(), CliError> {
    let mut cursor = since.unwrap_or(0);
    let follow = async {
        let mut initial = true;
        let mut seen = HashSet::new();
        let mut session_states = BTreeMap::new();
        loop {
            // Snapshot before reading so a terminal transition cannot skip its final events.
            let snapshot = follow_call(client, "mission_status", json!({"id": mission_id})).await?;
            let end_offset = snapshot.value["last_event_offset"].as_u64().unwrap_or(0);
            loop {
                let response = follow_call(
                    client,
                    "mission_feed",
                    json!({
                        "mission_id": mission_id,
                        "since_offset": if initial { since } else { Some(cursor) },
                        "limit": limit,
                        "order": if initial && !oldest_first { "newest_first" } else { "oldest_first" },
                    }),
                )
                .await?;
                let previous = cursor;
                write_follow_events(&response, filter, json, quiet, &mut seen, writer)?;
                if let Some(next) = response.value["next_offset"].as_u64() {
                    cursor = cursor.max(next);
                }
                initial = false;
                if cursor >= end_offset {
                    break;
                }
                if cursor == previous {
                    return Err(CliError {
                        code: 1,
                        message: "feed cursor did not advance to the mission snapshot".into(),
                    });
                }
            }
            if let Some(sessions) = snapshot.value["sessions"].as_array() {
                for session in sessions {
                    let id = field(session, "id").unwrap_or_default();
                    let status = field(session, "status").unwrap_or_default();
                    let previous = session_states.insert(id.to_owned(), status.to_owned());
                    if matches!(status, "stopped" | "crashed")
                        && previous.as_deref() != Some(status)
                    {
                        let label = field(session, "handle")
                            .map(|handle| format!("@{handle}"))
                            .unwrap_or_else(|| id.to_owned());
                        write_watch_notice(
                            diagnostics,
                            mission_id,
                            &format!("session {label} {status}"),
                        )?;
                    }
                }
            }
            if let Some((reason, failed)) = feed_end_reason(&snapshot.value) {
                if failed {
                    return Err(CliError {
                        code: 1,
                        message: reason.into(),
                    });
                }
                write_watch_notice(diagnostics, mission_id, reason)?;
                return Ok(());
            }
            tokio::time::sleep(FEED_POLL_INTERVAL).await;
        }
    };
    let result = tokio::select! {
        signal = tokio::signal::ctrl_c() => handle_ctrl_c(signal),
        result = follow => result,
    };
    result.map_err(|error| CliError {
        code: error.code,
        message: format!(
            "runner mission feed {mission_id}: watch ended: {}. Use the same Runner executable for `mission show {mission_id} --json`; if still active, resume with `mission feed {mission_id} --since {cursor} --oldest-first --follow --json`.",
            error.message
        ),
    })
}

async fn follow_call(
    client: &impl ToolCaller,
    tool: &str,
    args: Value,
) -> Result<ToolResponse, CliError> {
    tokio::time::timeout(FEED_REQUEST_TIMEOUT, call(client, tool, args))
        .await
        .map_err(|_| CliError {
            code: 1,
            message: "watch request timed out; Runner may still be running".into(),
        })?
}

fn feed_end_reason(snapshot: &Value) -> Option<(&'static str, bool)> {
    let mission = &snapshot["mission"];
    if field(mission, "status") == Some("aborted") {
        return Some(("mission aborted", true));
    }
    let sessions = snapshot["sessions"].as_array();
    let all_exited = sessions.is_some_and(|sessions| {
        !sessions.is_empty()
            && sessions
                .iter()
                .all(|session| matches!(field(session, "status"), Some("stopped" | "crashed")))
    });
    let archived = mission
        .get("archived_at")
        .is_some_and(|value| !value.is_null());
    let completed = field(mission, "status") == Some("completed");
    if (archived || completed || all_exited)
        && sessions.is_some_and(|sessions| {
            sessions
                .iter()
                .any(|session| field(session, "status") == Some("crashed"))
        })
    {
        Some(("mission watch ended with crashed sessions", true))
    } else if archived {
        Some(("mission archived; watch ended", false))
    } else if completed {
        Some(("mission completed; watch ended", false))
    } else if all_exited {
        Some((
            "all mission sessions exited; watch ended (arm a new watch if resumed)",
            false,
        ))
    } else {
        None
    }
}

fn write_watch_notice(
    writer: &mut impl std::io::Write,
    mission_id: &str,
    message: &str,
) -> Result<(), CliError> {
    writeln!(writer, "runner mission feed {mission_id}: {message}")
        .and_then(|()| writer.flush())
        .map_err(|error| CliError {
            code: 1,
            message: format!("watch diagnostic write failed: {error}"),
        })
}

fn handle_ctrl_c(result: std::io::Result<()>) -> Result<(), CliError> {
    result.map_err(|error| CliError {
        code: 1,
        message: format!("runner mission feed: Ctrl-C handler failed: {error}"),
    })
}

fn write_follow_events(
    response: &ToolResponse,
    filter: &FeedFilter,
    json: bool,
    quiet: bool,
    seen: &mut HashSet<String>,
    writer: &mut impl std::io::Write,
) -> Result<(), CliError> {
    let mut filtered = filter_feed_response(response, filter);
    if let Some(events) = filtered
        .value
        .get_mut("events")
        .and_then(Value::as_array_mut)
    {
        events.retain(|entry| {
            let event = entry.get("event").unwrap_or(entry);
            let key = field(event, "id")
                .map(str::to_owned)
                .or_else(|| entry.get("next_offset").map(Value::to_string))
                .unwrap_or_else(|| event.to_string());
            seen.insert(key)
        });
    }
    output::write_feed_events(&filtered.value, json, quiet, writer).map_err(|error| CliError {
        code: 1,
        message: format!("runner mission feed: write failed: {error}"),
    })
}

async fn run_chat(
    client: &impl ToolCaller,
    command: &ChatCommand,
) -> Result<ToolResponse, CliError> {
    let ChatCommand::Start {
        role,
        runtime,
        model,
        effort,
        project,
        cwd,
    } = command;
    let role_id = match role {
        Some(role) => Some(resolve_role(client, role).await?.id),
        None => None,
    };
    let project_id = match project {
        Some(project) => Some(
            resolve_named(client, "project", "project_list", project)
                .await?
                .id,
        ),
        None => None,
    };
    let cwd = if project_id.is_none() {
        Some(absolute_path(cwd.as_deref().unwrap_or(Path::new(".")))?)
    } else {
        None
    };
    call(
        client,
        "session_start_direct",
        json!({
            "role_id": role_id,
            "runtime": runtime,
            "model": model,
            "effort": effort,
            "project_id": project_id,
            "cwd": cwd,
        }),
    )
    .await
}

async fn run_session(
    client: &impl ToolCaller,
    command: &SessionCommand,
) -> Result<ToolResponse, CliError> {
    match command {
        SessionCommand::List => call(client, "session_list", json!({})).await,
        SessionCommand::Show { session }
        | SessionCommand::Stop { session }
        | SessionCommand::Archive { session } => {
            let session_id = resolve_session(client, session).await?;
            let tool = match command {
                SessionCommand::Show { .. } => "session_get",
                SessionCommand::Stop { .. } => "session_stop",
                SessionCommand::Archive { .. } => "session_archive",
                _ => unreachable!(),
            };
            call(client, tool, json!({"session_id": session_id})).await
        }
        SessionCommand::Resume { session } | SessionCommand::Restart { session } => {
            let id = resolve_session(client, session).await?;
            let tool = if matches!(command, SessionCommand::Resume { .. }) {
                "session_resume"
            } else {
                "session_restart"
            };
            call(client, tool, json!({"session_id": id})).await
        }
    }
}

async fn run_msg(
    client: &impl ToolCaller,
    command: &MsgCommand,
    context: &BusContext,
) -> Result<ToolResponse, CliError> {
    match command {
        MsgCommand::Post {
            text,
            to,
            mission,
            from,
        } => {
            let mission_id = resolve_scoped_mission(client, mission.as_deref(), context).await?;
            call(
                client,
                "mission_post",
                mission_post_args(&mission_id, text, to.as_deref(), from.as_deref()),
            )
            .await
        }
        MsgCommand::Read { .. } => unreachable!(),
    }
}

async fn run_ask(
    client: &impl ToolCaller,
    args: &AskArgs,
    context: &BusContext,
) -> Result<ToolResponse, CliError> {
    let mission_id = resolve_scoped_mission(client, args.mission.as_deref(), context).await?;
    let (signal_type, payload) = ask_signal(args)?;
    call(
        client,
        "mission_signal",
        mission_signal_args(&mission_id, signal_type, payload, args.from.as_deref()),
    )
    .await
}

fn ask_signal(args: &AskArgs) -> Result<(&'static str, Value), CliError> {
    match (&args.question, &args.human) {
        (Some(question), None) => {
            if args.choices.is_some() {
                return Err(CliError::usage("runner ask: --choices requires --human"));
            }
            let mut payload = json!({"question": question});
            insert_string(&mut payload, "context", args.context.as_deref());
            Ok(("ask_lead", payload))
        }
        (None, Some(prompt)) => {
            if args.context.is_some() {
                return Err(CliError::usage(
                    "runner ask: --context is for a lead question, not --human",
                ));
            }
            let choices = args
                .choices
                .as_deref()
                .ok_or_else(|| CliError::usage("runner ask --human requires --choices"))?
                .split(',')
                .map(str::trim)
                .filter(|choice| !choice.is_empty())
                .collect::<Vec<_>>();
            if choices.is_empty() {
                return Err(CliError::usage(
                    "runner ask --human requires at least one choice",
                ));
            }
            Ok(("ask_human", json!({"prompt": prompt, "choices": choices})))
        }
        _ => Err(CliError::usage(
            "runner ask requires either <question> or --human <prompt>",
        )),
    }
}

fn mission_post_args(mission_id: &str, text: &str, to: Option<&str>, from: Option<&str>) -> Value {
    let mut args = json!({"mission_id": mission_id, "text": text, "to": to});
    insert_string(&mut args, "from", from);
    args
}

fn mission_signal_args(
    mission_id: &str,
    signal_type: &str,
    payload: Value,
    from: Option<&str>,
) -> Value {
    let mut args = json!({
        "mission_id": mission_id,
        "signal_type": signal_type,
        "payload": payload,
    });
    insert_string(&mut args, "from", from);
    args
}

async fn mission_lifecycle(
    client: &impl ToolCaller,
    tool: &str,
    mission: Option<&str>,
    context: &BusContext,
) -> Result<ToolResponse, CliError> {
    let id = resolve_mission_arg(client, mission, context).await?;
    let mut response = call(client, tool, json!({"id": id})).await?;
    if tool == "mission_resume" {
        let mission = call(client, "mission_get", json!({"id": id})).await?;
        response.value = mission.value;
    } else if tool == "mission_stop" {
        if let Some(mission) = mission_value_mut(&mut response.value).as_object_mut() {
            mission.insert("status".into(), Value::String("stopped".into()));
        }
    }
    let crew_id = mission_value(&response.value)
        .get("crew_id")
        .and_then(Value::as_str)
        .map(str::to_owned);
    if let Some(crew_id) = crew_id {
        if let Ok(crews) = list(client, "crew_list", json!({})).await {
            if let Some(name) = crews
                .iter()
                .find(|crew| field(crew, "id") == Some(&crew_id))
                .and_then(|crew| field(crew, "name"))
                .map(str::to_owned)
            {
                let mission = mission_value_mut(&mut response.value);
                if let Some(mission) = mission.as_object_mut() {
                    mission.insert("crew_name".into(), Value::String(name));
                }
            }
        }
    }
    Ok(response)
}

fn mission_value(value: &Value) -> &Value {
    value.get("mission").unwrap_or(value)
}

fn mission_value_mut(value: &mut Value) -> &mut Value {
    if value.get("mission").is_some() {
        value.get_mut("mission").expect("mission exists")
    } else {
        value
    }
}

async fn resolve_scoped_mission(
    client: &impl ToolCaller,
    target: Option<&str>,
    context: &BusContext,
) -> Result<String, CliError> {
    match target {
        Some(target) => resolve_mission(client, target).await,
        None => match context {
            BusContext::Mission(mission) => Ok(mission.mission_id.clone()),
            BusContext::OffBus => Err(CliError::usage(
                "runner: --mission is required outside a mission",
            )),
            BusContext::Partial { .. } => unreachable!(),
        },
    }
}

async fn resolve_mission_arg(
    client: &impl ToolCaller,
    target: Option<&str>,
    context: &BusContext,
) -> Result<String, CliError> {
    match target {
        Some(target) => resolve_mission(client, target).await,
        None => match context {
            BusContext::Mission(mission) => Ok(mission.mission_id.clone()),
            BusContext::OffBus => Err(CliError::usage("mission reference is required")),
            BusContext::Partial { .. } => unreachable!(),
        },
    }
}

async fn resolve_mission(client: &impl ToolCaller, target: &str) -> Result<String, CliError> {
    match resolve_prefix(client, "mission", "mission_list", target).await {
        Ok(id) => Ok(id),
        Err(error) if target.len() == 26 && error.code == 2 => {
            match call(client, "mission_get", json!({"id": target})).await {
                Ok(_) => Ok(target.to_owned()),
                Err(fallback) if fallback.code == 1 => Err(error),
                Err(fallback) => Err(fallback),
            }
        }
        Err(error) => Err(error),
    }
}

#[derive(Debug, Clone)]
struct Resolved {
    id: String,
    name: String,
}

async fn resolve_named(
    client: &impl ToolCaller,
    kind: &str,
    tool: &str,
    target: &str,
) -> Result<Resolved, CliError> {
    let rows = list(client, tool, json!({})).await?;
    resolve_named_rows(&rows, kind, target)
}

fn resolve_named_rows(rows: &[Value], kind: &str, target: &str) -> Result<Resolved, CliError> {
    if let Some(row) = rows.iter().find(|row| field(row, "id") == Some(target)) {
        return Ok(Resolved {
            id: target.to_owned(),
            name: field(row, "name").unwrap_or(target).to_owned(),
        });
    }
    let matches = rows
        .iter()
        .filter(|row| field(row, "name") == Some(target))
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [row] => Ok(Resolved {
            id: field(row, "id").unwrap().to_owned(),
            name: target.to_owned(),
        }),
        [] => Err(CliError::usage(format!("{kind} not found: {target}"))),
        rows => Err(CliError::usage(format!(
            "ambiguous {kind} name {target:?}: {}",
            rows.iter()
                .filter_map(|row| field(row, "id"))
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}

async fn resolve_role(client: &impl ToolCaller, handle: &str) -> Result<Resolved, CliError> {
    let rows = list(client, "role_list", json!({})).await?;
    resolve_role_rows(&rows, handle)
}

async fn resolve_session(client: &impl ToolCaller, target: &str) -> Result<String, CliError> {
    match resolve_prefix(client, "session", "session_list", target).await {
        Ok(id) => Ok(id),
        Err(error) if target.len() == 26 && error.code == 2 => Ok(target.to_owned()),
        Err(error) => Err(error),
    }
}

fn resolve_role_rows(rows: &[Value], handle: &str) -> Result<Resolved, CliError> {
    let handle = handle.strip_prefix('@').unwrap_or(handle);
    let row = rows
        .iter()
        .find(|row| field(row, "handle") == Some(handle))
        .ok_or_else(|| CliError::usage(format!("role handle not found: @{handle}")))?;
    Ok(Resolved {
        id: field(row, "id").unwrap().to_owned(),
        name: handle.to_owned(),
    })
}

async fn resolve_slot(
    client: &impl ToolCaller,
    crew_id: &str,
    handle: &str,
) -> Result<Resolved, CliError> {
    let rows = list(client, "slot_list", json!({"crew_id": crew_id})).await?;
    let row = rows
        .iter()
        .find(|row| field(row, "slot_handle") == Some(handle))
        .ok_or_else(|| CliError::usage(format!("crew slot handle not found: {handle}")))?;
    Ok(Resolved {
        id: field(row, "id").unwrap().to_owned(),
        name: handle.to_owned(),
    })
}

async fn resolve_prefix(
    client: &impl ToolCaller,
    kind: &str,
    tool: &str,
    target: &str,
) -> Result<String, CliError> {
    let rows = list(client, tool, json!({})).await?;
    resolve_prefix_rows(&rows, kind, target)
}

fn resolve_prefix_rows(rows: &[Value], kind: &str, target: &str) -> Result<String, CliError> {
    if rows.iter().any(|row| reference_id(row) == Some(target)) {
        return Ok(target.to_owned());
    }
    let matches = rows
        .iter()
        .filter_map(reference_id)
        .filter(|id| id.starts_with(target))
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [id] => Ok((*id).to_owned()),
        [] => Err(CliError::usage(format!("{kind} not found: {target}"))),
        ids => Err(CliError::usage(format!(
            "ambiguous {kind} id prefix {target:?}: {}",
            ids.join(", ")
        ))),
    }
}

fn reference_id(value: &Value) -> Option<&str> {
    field(value, "id").or_else(|| field(value, "session_id"))
}

async fn list(client: &impl ToolCaller, tool: &str, args: Value) -> Result<Vec<Value>, CliError> {
    let response = call(client, tool, args).await?;
    response.value.as_array().cloned().ok_or_else(|| CliError {
        code: 1,
        message: format!("{tool} returned a non-list result"),
    })
}

async fn call(client: &impl ToolCaller, tool: &str, args: Value) -> Result<ToolResponse, CliError> {
    client.call(tool, args).await
}

fn role_fields(fields: &RoleFields, update: bool) -> Result<Value, CliError> {
    let mut value = json!({});
    if update {
        insert_string(&mut value, "display_name", fields.name.as_deref());
        insert_string(&mut value, "runtime", fields.runtime.as_deref());
        if let Some(runtime) = fields.runtime.as_deref() {
            insert_string(&mut value, "command", Some(runtime_command(runtime)?));
        }
    }
    insert_string(&mut value, "model", fields.model.as_deref());
    insert_string(&mut value, "effort", fields.effort.as_deref());
    insert_string(&mut value, "permission_mode", fields.permission.as_deref());
    let prompt = read_text(
        fields.prompt.as_deref(),
        fields.prompt_file.as_deref(),
        "prompt",
    )?;
    if let Some(prompt) = prompt {
        insert_value(&mut value, "system_prompt", json!(prompt));
    }
    if !fields.args.is_empty() {
        insert_value(&mut value, "args", json!(fields.args));
    } else if !update {
        insert_value(&mut value, "args", json!([]));
    }
    let env = parse_env(&fields.env)?;
    if !env.is_empty() || !update {
        insert_value(&mut value, "env", json!(env));
    }
    insert_string(&mut value, "working_dir", fields.cwd.as_deref());
    Ok(value)
}

fn crew_fields(fields: &CrewFields) -> Result<Value, CliError> {
    let mut value = json!({});
    insert_string(&mut value, "name", fields.name.as_deref());
    insert_string(&mut value, "purpose", fields.purpose.as_deref());
    insert_string(&mut value, "goal", fields.goal.as_deref());
    if let Some(path) = fields.conventions_file.as_deref() {
        let text = read_file(path, "conventions")?;
        insert_value(&mut value, "system_prompt_addendum", json!(text));
    }
    Ok(value)
}

fn crew_create_fields(fields: &CrewCreateFields) -> Result<Value, CliError> {
    let mut value = json!({});
    insert_clearable(&mut value, "purpose", fields.purpose.as_deref(), false);
    insert_clearable(&mut value, "goal", fields.goal.as_deref(), false);
    if let Some(path) = fields.conventions_file.as_deref() {
        insert_value(
            &mut value,
            "system_prompt_addendum",
            json!(read_file(path, "conventions")?),
        );
    }
    Ok(value)
}

fn runtime_command(runtime: &str) -> Result<&'static str, CliError> {
    match runtime {
        "claude-code" => Ok("claude"),
        "codex" => Ok("codex"),
        "trae" => Ok("traecli"),
        "copilot" => Ok("copilot"),
        "pi" => Ok("pi"),
        other => Err(CliError::usage(format!(
            "unknown role runtime {other:?}; expected claude-code, codex, trae, copilot, or pi"
        ))),
    }
}

fn validate_permission(permission: Option<&str>) -> Result<(), CliError> {
    match permission {
        None | Some("default" | "accept_edits" | "auto" | "bypass") => Ok(()),
        Some(other) => Err(CliError::usage(format!(
            "unknown permission mode {other:?}; expected default, accept_edits, auto, or bypass"
        ))),
    }
}

fn parse_env(values: &[String]) -> Result<BTreeMap<String, String>, CliError> {
    values
        .iter()
        .map(|value| {
            value
                .split_once('=')
                .filter(|(key, _)| !key.is_empty())
                .map(|(key, value)| (key.to_owned(), value.to_owned()))
                .ok_or_else(|| {
                    CliError::usage(format!("invalid --env {value:?}; expected KEY=VALUE"))
                })
        })
        .collect()
}

fn read_text(
    inline: Option<&str>,
    file: Option<&Path>,
    label: &str,
) -> Result<Option<String>, CliError> {
    if let Some(value) = inline {
        Ok(Some(value.to_owned()))
    } else if let Some(path) = file {
        read_file(path, label).map(Some)
    } else {
        Ok(None)
    }
}

fn read_file(path: &Path, label: &str) -> Result<String, CliError> {
    if path == Path::new("-") {
        let mut text = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut text).map_err(|error| {
            CliError::usage(format!("failed to read {label} from stdin: {error}"))
        })?;
        Ok(text)
    } else {
        std::fs::read_to_string(path).map_err(|error| {
            CliError::usage(format!(
                "failed to read {label} file {}: {error}",
                path.display()
            ))
        })
    }
}

fn absolute_path(path: &Path) -> Result<String, CliError> {
    let path = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()
            .map_err(|error| CliError::usage(format!("failed to read current directory: {error}")))?
            .join(path)
    };
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            component => normalized.push(component.as_os_str()),
        }
    }
    Ok(normalized.to_string_lossy().into_owned())
}

fn parse_json_object(value: &str) -> Result<Value, CliError> {
    let value: Value = serde_json::from_str(value)
        .map_err(|error| CliError::usage(format!("invalid JSON arguments: {error}")))?;
    if value.is_object() {
        Ok(value)
    } else {
        Err(CliError::usage("tool arguments must be a JSON object"))
    }
}

fn response(value: Value) -> Result<ToolResponse, CliError> {
    let raw_json = serde_json::to_string(&value).map_err(|error| CliError {
        code: 1,
        message: error.to_string(),
    })?;
    Ok(ToolResponse { value, raw_json })
}

fn field<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn id_from(value: &Value) -> Result<String, CliError> {
    ["id", "slot_id", "session_id"]
        .iter()
        .find_map(|key| field(value, key))
        .map(str::to_owned)
        .ok_or_else(|| CliError {
            code: 1,
            message: "tool result did not contain an id".to_owned(),
        })
}

fn insert_value(target: &mut Value, key: &str, value: Value) {
    target
        .as_object_mut()
        .expect("command arguments are objects")
        .insert(key.to_owned(), value);
}

fn insert_string(target: &mut Value, key: &str, value: Option<&str>) {
    if let Some(value) = value {
        insert_value(target, key, json!(value));
    }
}

fn insert_clearable(target: &mut Value, key: &str, value: Option<&str>, clear: bool) {
    if let Some(value) = value {
        insert_value(
            target,
            key,
            if clear { nullable(value) } else { json!(value) },
        );
    }
}

fn nullable(value: &str) -> Value {
    if value.is_empty() {
        Value::Null
    } else {
        json!(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    #[derive(Default)]
    struct RecordingClient {
        calls: Mutex<Vec<(String, Value)>>,
    }

    struct FailingMissionGetClient {
        code: i32,
    }

    struct SequenceClient {
        responses: Mutex<VecDeque<(String, Result<Value, CliError>)>>,
        calls: Mutex<Vec<(String, Value)>>,
    }

    impl SequenceClient {
        fn new(responses: Vec<(&str, Result<Value, CliError>)>) -> Self {
            Self {
                responses: Mutex::new(
                    responses
                        .into_iter()
                        .map(|(name, response)| (name.to_owned(), response))
                        .collect(),
                ),
                calls: Mutex::new(Vec::new()),
            }
        }
    }

    impl ToolCaller for RecordingClient {
        async fn call(&self, name: &str, arguments: Value) -> Result<ToolResponse, CliError> {
            self.calls
                .lock()
                .unwrap()
                .push((name.to_owned(), arguments));
            let value = match name {
                "project_list" => json!([{"id": "project-id", "name": "Runner"}]),
                "role_list" => json!([{"id": "role-id", "handle": "coder"}]),
                "crew_list" => json!([{"id": "crew-id", "name": "Peer"}]),
                "slot_list" => json!([
                    {"id": "slot-lead", "slot_handle": "lead"},
                    {"id": "slot-impl", "slot_handle": "impl"}
                ]),
                "mission_list" => json!([{"id": "01M00000000000000000000000"}]),
                "session_list" => json!([{"session_id": "01S00000000000000000000000"}]),
                "slot_create" => json!({"id": "slot-new"}),
                "mission_get" | "mission_status" | "crew_get" | "role_get" => {
                    json!({"id": "object-id"})
                }
                _ => json!({"id": "changed-id"}),
            };
            response(value)
        }
    }

    impl ToolCaller for FailingMissionGetClient {
        async fn call(&self, name: &str, _arguments: Value) -> Result<ToolResponse, CliError> {
            match name {
                "mission_list" => response(json!([])),
                "mission_get" => Err(CliError {
                    code: self.code,
                    message: "mission does not exist".into(),
                }),
                _ => panic!("unexpected tool {name}"),
            }
        }
    }

    impl ToolCaller for SequenceClient {
        async fn call(&self, name: &str, arguments: Value) -> Result<ToolResponse, CliError> {
            self.calls
                .lock()
                .unwrap()
                .push((name.to_owned(), arguments));
            let (expected, response_value) = self
                .responses
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| panic!("unexpected tool call {name}"));
            assert_eq!(name, expected);
            response_value.and_then(response)
        }
    }

    fn feed_event(id: &str, next_offset: u64, kind: &str, from: &str) -> Value {
        json!({
            "next_offset": next_offset,
            "event": {
                "id": id,
                "ts": format!("2026-09-18T10:20:{id}Z"),
                "kind": if kind == "message" { "message" } else { "signal" },
                "from": from,
                "to": null,
                "type": if kind == "message" { Value::Null } else { json!(kind) },
                "payload": {"text": id},
            }
        })
    }

    fn feed_snapshot(status: &str, sessions: &[(&str, &str)], offset: u64) -> Value {
        json!({
            "mission": {"status": status, "archived_at": null},
            "sessions": sessions.iter().enumerate().map(|(index, (handle, status))| json!({
                "id": format!("{:026}", index + 1), "handle": handle, "status": status,
            })).collect::<Vec<_>>(),
            "last_event_offset": offset,
        })
    }

    #[test]
    fn feed_poll_interval_is_three_seconds_with_a_fast_test_seam() {
        assert_eq!(DEFAULT_FEED_POLL_INTERVAL, Duration::from_secs(3));
        assert_eq!(FEED_POLL_INTERVAL, Duration::from_millis(1));
        assert_eq!(DEFAULT_FEED_REQUEST_TIMEOUT, Duration::from_secs(30));
        assert!(DEFAULT_FEED_REQUEST_TIMEOUT > DEFAULT_FEED_POLL_INTERVAL);
    }

    #[test]
    fn follow_rejects_zero_limit_without_changing_one_shot_feed() {
        let args = ["runner", "mission", "feed", "mission", "--limit", "0"];
        let one_shot = Cli::try_parse_from(args).unwrap();
        validate_remote(&one_shot, &BusContext::OffBus).unwrap();
        let follow = Cli::try_parse_from(args.into_iter().chain(["--follow"])).unwrap();
        let error = validate_remote(&follow, &BusContext::OffBus).unwrap_err();
        assert_eq!(error.code, 2);
        assert!(error.message.contains("positive --limit"));
    }

    #[tokio::test]
    async fn follow_prints_each_event_once_across_empty_and_multi_event_polls() {
        let e1 = feed_event("01", 10, "message", "human");
        let e2 = feed_event("02", 20, "ask_lead", "coder");
        let e3 = feed_event("03", 30, "message", "reviewer");
        let e4 = feed_event("04", 40, "human_question", "human");
        let running = feed_snapshot("running", &[("coder", "running")], 10);
        let mut archived = feed_snapshot("completed", &[("coder", "stopped")], 40);
        archived["mission"]["archived_at"] = json!("2026-09-18T10:21:00Z");
        let client = SequenceClient::new(vec![
            ("mission_status", Ok(running.clone())),
            (
                "mission_feed",
                Ok(json!({"events": [e1.clone()], "next_offset": 10})),
            ),
            ("mission_status", Ok(running.clone())),
            (
                "mission_feed",
                Ok(json!({"events": [], "next_offset": null})),
            ),
            (
                "mission_status",
                Ok(feed_snapshot("running", &[("coder", "running")], 20)),
            ),
            (
                "mission_feed",
                Ok(json!({"events": [e1, e2], "next_offset": 20})),
            ),
            (
                "mission_status",
                Ok(feed_snapshot("running", &[("coder", "running")], 40)),
            ),
            (
                "mission_feed",
                Ok(json!({"events": [e3, e4], "next_offset": 40})),
            ),
            ("mission_status", Ok(archived)),
            (
                "mission_feed",
                Ok(json!({"events": [], "next_offset": null})),
            ),
        ]);
        let mut output = Vec::new();
        let mut diagnostics = Vec::new();
        follow_feed(
            &client,
            "mission",
            None,
            None,
            false,
            &FeedFilter::new(None, None, false).unwrap(),
            true,
            false,
            &mut output,
            &mut diagnostics,
        )
        .await
        .unwrap();
        let lines = String::from_utf8(output)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        let ids = lines
            .iter()
            .map(|line| line["id"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(ids, ["01", "02", "03", "04"]);
        let resume_offsets = lines
            .iter()
            .map(|line| line["next_offset"].as_u64().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(resume_offsets, [10, 20, 30, 40]);
        assert!(String::from_utf8(diagnostics)
            .unwrap()
            .contains("mission archived; watch ended"));
        let calls = client.calls.into_inner().unwrap();
        let cursors = calls
            .iter()
            .filter(|(name, _)| name == "mission_feed")
            .skip(1)
            .map(|(_, args)| args["since_offset"].as_u64().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(cursors, [10, 10, 20, 40]);
    }

    #[tokio::test]
    async fn follow_drains_terminal_snapshot_pages_before_exiting() {
        let client = SequenceClient::new(vec![
            ("mission_status", Ok(feed_snapshot("completed", &[], 30))),
            (
                "mission_feed",
                Ok(
                    json!({"events": [feed_event("01", 10, "message", "coder")], "next_offset": 10}),
                ),
            ),
            (
                "mission_feed",
                Ok(
                    json!({"events": [feed_event("02", 20, "human_question", "coder")], "next_offset": 20}),
                ),
            ),
            (
                "mission_feed",
                Ok(
                    json!({"events": [feed_event("03", 30, "mission_stopped", "system")], "next_offset": 30}),
                ),
            ),
        ]);
        let mut output = Vec::new();
        follow_feed(
            &client,
            "mission",
            Some(0),
            Some(1),
            true,
            &FeedFilter::new(None, None, false).unwrap(),
            true,
            false,
            &mut output,
            &mut Vec::new(),
        )
        .await
        .unwrap();
        let lines = String::from_utf8(output).unwrap();
        assert_eq!(lines.lines().count(), 3);
        assert!(lines.contains("human_question"));
        assert!(lines.contains("mission_stopped"));
        let calls = client.calls.into_inner().unwrap();
        assert_eq!(calls.len(), 4);
        assert_eq!(calls[3].1["since_offset"], 20);
    }

    #[tokio::test]
    async fn follow_reports_partial_crash_once_then_stops_when_all_sessions_exit() {
        let running = feed_snapshot(
            "running",
            &[("coder", "crashed"), ("reviewer", "running")],
            0,
        );
        let ended = feed_snapshot(
            "running",
            &[("coder", "crashed"), ("reviewer", "stopped")],
            0,
        );
        let client = SequenceClient::new(vec![
            ("mission_status", Ok(running.clone())),
            ("mission_feed", Ok(json!({"events": []}))),
            ("mission_status", Ok(running)),
            ("mission_feed", Ok(json!({"events": []}))),
            ("mission_status", Ok(ended)),
            ("mission_feed", Ok(json!({"events": []}))),
        ]);
        let mut diagnostics = Vec::new();
        let error = follow_feed(
            &client,
            "mission",
            None,
            None,
            false,
            &FeedFilter::new(Some("message"), Some("reviewer"), false).unwrap(),
            true,
            false,
            &mut Vec::new(),
            &mut diagnostics,
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, 1);
        assert!(error.message.contains("crashed sessions"));
        let notices = String::from_utf8(diagnostics).unwrap();
        assert_eq!(notices.matches("session @coder crashed").count(), 1);
        assert!(notices.contains("session @reviewer stopped"));
    }

    #[tokio::test]
    async fn follow_exits_on_stop_completion_and_abort_without_archive() {
        for (status, sessions, reason, code) in [
            (
                "running",
                vec![("coder", "stopped")],
                "all mission sessions exited",
                0,
            ),
            ("completed", vec![], "mission completed", 0),
            ("aborted", vec![], "mission aborted", 1),
        ] {
            let client = SequenceClient::new(vec![
                ("mission_status", Ok(feed_snapshot(status, &sessions, 0))),
                ("mission_feed", Ok(json!({"events": []}))),
            ]);
            let mut diagnostics = Vec::new();
            let result = follow_feed(
                &client,
                "mission",
                None,
                None,
                false,
                &FeedFilter::new(None, None, false).unwrap(),
                true,
                false,
                &mut Vec::new(),
                &mut diagnostics,
            )
            .await;
            if code == 0 {
                result.unwrap();
                assert!(String::from_utf8(diagnostics).unwrap().contains(reason));
            } else {
                let error = result.unwrap_err();
                assert_eq!(error.code, code);
                assert!(error.message.contains(reason));
            }
        }
        assert_eq!(feed_end_reason(&feed_snapshot("running", &[], 0)), None);
        assert_eq!(
            feed_end_reason(&feed_snapshot("running", &[("coder", "running")], 0)),
            None
        );
    }

    #[tokio::test]
    async fn follow_reports_app_disconnect_and_missing_mission_with_recovery_cursor() {
        for (tool, code, message) in [
            ("mission_status", 3, runner_cli::client::NOT_RUNNING_MESSAGE),
            ("mission_status", 1, "mission does not exist"),
            ("mission_feed", 3, runner_cli::client::NOT_RUNNING_MESSAGE),
            ("mission_feed", 1, "mission does not exist"),
        ] {
            let mut responses = vec![
                ("mission_status", Ok(feed_snapshot("running", &[], 10))),
                (
                    "mission_feed",
                    Ok(
                        json!({"events": [feed_event("01", 10, "message", "coder")], "next_offset": 10}),
                    ),
                ),
            ];
            if tool == "mission_feed" {
                responses.push(("mission_status", Ok(feed_snapshot("running", &[], 10))));
            }
            responses.push((
                tool,
                Err(CliError {
                    code,
                    message: message.into(),
                }),
            ));
            let client = SequenceClient::new(responses);
            let error = follow_feed(
                &client,
                "mission",
                None,
                None,
                false,
                &FeedFilter::new(None, None, false).unwrap(),
                false,
                false,
                &mut Vec::new(),
                &mut Vec::new(),
            )
            .await
            .unwrap_err();
            assert_eq!(error.code, code);
            assert!(error.message.contains(message));
            assert!(error.message.contains("watch ended"));
            assert!(error.message.contains("Use the same Runner executable"));
            assert!(error
                .message
                .contains("mission feed mission --since 10 --oldest-first --follow --json"));
        }
    }

    #[tokio::test]
    async fn follow_reports_an_unresponsive_app() {
        struct UnresponsiveClient;
        impl ToolCaller for UnresponsiveClient {
            async fn call(&self, _name: &str, _arguments: Value) -> Result<ToolResponse, CliError> {
                std::future::pending().await
            }
        }
        let error = follow_feed(
            &UnresponsiveClient,
            "mission",
            None,
            None,
            false,
            &FeedFilter::new(None, None, false).unwrap(),
            true,
            false,
            &mut Vec::new(),
            &mut Vec::new(),
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, 1);
        assert!(error.message.contains("timed out"));
        assert!(error.message.contains("Runner may still be running"));
    }

    #[tokio::test]
    async fn follow_reports_a_broken_cursor_instead_of_spinning() {
        let client = SequenceClient::new(vec![
            ("mission_status", Ok(feed_snapshot("completed", &[], 10))),
            (
                "mission_feed",
                Ok(json!({"events": [], "next_offset": null})),
            ),
        ]);
        let error = follow_feed(
            &client,
            "mission",
            None,
            None,
            true,
            &FeedFilter::new(None, None, false).unwrap(),
            true,
            false,
            &mut Vec::new(),
            &mut Vec::new(),
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, 1);
        assert!(error.message.contains("cursor did not advance"));
    }

    #[test]
    fn default_feed_filter_preserves_real_failed_status_payloads() {
        for kind in ["session_status", "runner_status"] {
            for state in ["busy", "idle"] {
                for (lifecycle, outcome, visible) in [
                    ("running", "failed", true),
                    ("error", "completed", true),
                    ("running", "completed", false),
                    ("running", "interrupted", false),
                ] {
                    let mut event = feed_event("01", 10, kind, "coder");
                    event["event"]["payload"] = json!({
                        "state": state,
                        "source": "hook",
                        "status": {
                            "lifecycle": lifecycle,
                            "observation": {
                                "activity": if state == "busy" { "working" } else { "idle" },
                                "source": "hook",
                                "outcome": outcome,
                                "interactions": [],
                                "detail": null,
                            },
                            "exit_code": null,
                            "error_since": null,
                            "failed_since": null,
                            "unread_since": null,
                        },
                    });
                    assert_eq!(
                        FeedFilter::new(None, None, false).unwrap().matches(&event),
                        visible
                    );
                }
            }
        }
    }

    #[tokio::test]
    async fn follow_allows_a_response_slower_than_the_poll_interval() {
        struct SlowClient;
        impl ToolCaller for SlowClient {
            async fn call(&self, name: &str, _arguments: Value) -> Result<ToolResponse, CliError> {
                tokio::time::sleep(FEED_POLL_INTERVAL * 2).await;
                match name {
                    "mission_status" => response(feed_snapshot("completed", &[], 0)),
                    "mission_feed" => response(json!({"events": []})),
                    _ => panic!("unexpected tool {name}"),
                }
            }
        }
        follow_feed(
            &SlowClient,
            "mission",
            None,
            None,
            false,
            &FeedFilter::new(None, None, false).unwrap(),
            true,
            false,
            &mut Vec::new(),
            &mut Vec::new(),
        )
        .await
        .unwrap();
    }

    #[test]
    fn feed_filters_types_senders_and_default_noise() {
        let message = feed_event("01", 10, "message", "human");
        let status = feed_event("02", 20, "session_status", "coder");
        let inbox = feed_event("03", 30, "inbox_read", "coder");
        let ask = feed_event("04", 40, "ask_lead", "reviewer");
        let legacy = feed_event("05", 50, "runner_status", "coder");
        let response = response(json!({
            "events": [message, status, inbox, ask, legacy],
            "next_offset": 50,
            "skipped": []
        }))
        .unwrap();

        let default = filter_feed_response(&response, &FeedFilter::new(None, None, false).unwrap());
        assert_eq!(default.value["events"].as_array().unwrap().len(), 2);
        let all = filter_feed_response(&response, &FeedFilter::new(None, None, true).unwrap());
        assert_eq!(all.value["events"].as_array().unwrap().len(), 5);
        let statuses = filter_feed_response(
            &response,
            &FeedFilter::new(Some("session_status,inbox_read"), None, false).unwrap(),
        );
        assert_eq!(statuses.value["events"].as_array().unwrap().len(), 2);
        let reviewer = filter_feed_response(
            &response,
            &FeedFilter::new(None, Some("reviewer"), false).unwrap(),
        );
        assert_eq!(reviewer.value["events"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn plain_json_feed_without_filters_stays_verbatim() {
        let cli = Cli::try_parse_from(["runner", "mission", "feed", "01M", "--json"]).unwrap();
        let response = ToolResponse {
            value: json!({"events": [], "next_offset": null}),
            raw_json: "{ \"events\" : [], \"next_offset\" : null }".into(),
        };
        assert_eq!(
            postprocess_response(&cli, response).raw_json,
            "{ \"events\" : [], \"next_offset\" : null }"
        );
    }

    #[test]
    fn runner_status_classifies_managed_foreign_and_missing_skill_roots() {
        let home = tempfile::tempdir().unwrap();
        let managed = home.path().join(".claude/skills/runner-dev");
        std::fs::create_dir_all(&managed).unwrap();
        std::fs::write(managed.join(runner_core::RUNNER_SKILL_MARKER), "managed").unwrap();
        let foreign = home.path().join(".agents/skills/runner-dev");
        std::fs::create_dir_all(&foreign).unwrap();

        let statuses = skill_statuses(home.path(), true);

        assert_eq!(statuses[0]["state"], "managed");
        assert_eq!(statuses[1]["state"], "foreign");
        assert_eq!(statuses[2]["state"], "missing");
    }

    #[test]
    fn updates_send_empty_strings_for_backend_clears() {
        let fields = RoleFields {
            runtime: Some("trae".into()),
            model: Some(String::new()),
            effort: Some(String::new()),
            prompt: Some(String::new()),
            cwd: Some(String::new()),
            ..Default::default()
        };
        assert_eq!(
            runtime_command(fields.runtime.as_deref().unwrap()).unwrap(),
            "traecli"
        );
        let update = role_fields(&fields, true).unwrap();
        assert_eq!(update["model"], json!(""));
        assert_eq!(update["effort"], json!(""));
        assert_eq!(update["system_prompt"], json!(""));
        assert_eq!(update["working_dir"], json!(""));

        let fields = CrewFields {
            purpose: Some(String::new()),
            goal: Some(String::new()),
            ..Default::default()
        };
        let update = crew_fields(&fields).unwrap();
        assert_eq!(update["purpose"], json!(""));
        assert_eq!(update["goal"], json!(""));
    }

    #[test]
    fn ask_builds_both_payload_shapes() {
        let lead = AskArgs {
            question: Some("Ship?".into()),
            context: Some("Tests pass".into()),
            human: None,
            choices: None,
            mission: None,
            from: None,
        };
        assert_eq!(
            ask_signal(&lead).unwrap(),
            (
                "ask_lead",
                json!({"question": "Ship?", "context": "Tests pass"})
            )
        );
        let human = AskArgs {
            question: None,
            context: None,
            human: Some("Ship?".into()),
            choices: Some("yes, no".into()),
            mission: None,
            from: None,
        };
        assert_eq!(
            ask_signal(&human).unwrap(),
            (
                "ask_human",
                json!({"prompt": "Ship?", "choices": ["yes", "no"]})
            )
        );
    }

    #[test]
    fn outside_identity_is_omitted_for_the_person_and_forwarded_for_a_handle() {
        assert_eq!(
            mission_post_args("mission", "hello", Some("lead"), None),
            json!({"mission_id": "mission", "text": "hello", "to": "lead"})
        );
        assert_eq!(
            mission_post_args("mission", "hello", None, Some("coder"))["from"],
            "coder"
        );
        assert_eq!(
            mission_signal_args("mission", "ask_lead", json!({}), Some("coder")),
            json!({
                "mission_id": "mission",
                "from": "coder",
                "signal_type": "ask_lead",
                "payload": {},
            })
        );
        assert!(mission_signal_args(
            "mission",
            "human_response",
            json!({"question_id": "q", "choice": "yes"}),
            None,
        )
        .get("from")
        .is_none());
    }

    #[test]
    fn call_requires_object_arguments() {
        assert!(parse_json_object("[]").is_err());
        assert_eq!(
            parse_json_object(r#"{"id":"x"}"#).unwrap(),
            json!({"id": "x"})
        );
    }

    #[test]
    fn client_failures_map_to_documented_exit_codes() {
        assert_eq!(CliError::from(ClientError::Refused("no".into())).code, 1);
        assert_eq!(CliError::from(ClientError::NotRunning).code, 3);
        let blocked = CliError::from(ClientError::Blocked);
        assert_eq!(blocked.code, 5);
        assert_eq!(blocked.message, runner_cli::client::BLOCKED_MESSAGE);
    }

    #[test]
    fn references_resolve_handles_names_and_prefixes() {
        let roles = vec![json!({"id": "role-id", "handle": "coder"})];
        assert_eq!(resolve_role_rows(&roles, "@coder").unwrap().id, "role-id");

        let crews = vec![
            json!({"id": "crew-a", "name": "Peer"}),
            json!({"id": "crew-b", "name": "Solo"}),
        ];
        assert_eq!(
            resolve_named_rows(&crews, "crew", "Peer").unwrap().id,
            "crew-a"
        );
        let ambiguous = vec![
            json!({"id": "crew-a", "name": "Peer"}),
            json!({"id": "crew-b", "name": "Peer"}),
        ];
        let error = resolve_named_rows(&ambiguous, "crew", "Peer").unwrap_err();
        assert_eq!(error.code, 2);
        assert!(error.message.contains("crew-a") && error.message.contains("crew-b"));

        let missions = vec![
            json!({"id": "01MISSIONAAAA"}),
            json!({"id": "01MISSIONBBBB"}),
        ];
        assert_eq!(
            resolve_prefix_rows(&missions, "mission", "01MISSIONA").unwrap(),
            "01MISSIONAAAA"
        );
        let error = resolve_prefix_rows(&missions, "mission", "01MISSION").unwrap_err();
        assert_eq!(error.code, 2);
        assert!(error.message.contains("01MISSIONAAAA"));
        assert!(error.message.contains("01MISSIONBBBB"));
    }

    #[test]
    fn own_mission_prefix_stays_on_the_local_event_log_path() {
        let mission = MissionEnv {
            crew_id: "crew".into(),
            mission_id: "01M00000000000000000000000".into(),
            handle: "coder".into(),
            event_log: PathBuf::from("events.ndjson"),
        };
        assert!(same_mission(None, &mission));
        assert!(same_mission(Some("01M"), &mission));
        assert!(!same_mission(Some("01A"), &mission));
    }

    #[test]
    fn paths_are_made_absolute_and_normalized_without_filesystem_access() {
        let cwd = std::env::current_dir().unwrap();
        assert_eq!(
            absolute_path(Path::new(".")).unwrap(),
            cwd.to_string_lossy()
        );
        assert_eq!(
            absolute_path(Path::new("./x/../y")).unwrap(),
            cwd.join("y").to_string_lossy()
        );
    }

    #[test]
    fn command_status_path_filter_drops_only_runner_app_data_entries() {
        assert_eq!(
            path_without_runner_entries(
                "/app/data/missions/id/shims/coder/bin:/app/data/bin:/usr/local/bin",
                Path::new("/app/data"),
                runner_core::command_install::PathStyle::Unix,
            ),
            "/usr/local/bin"
        );
        assert_eq!(
            path_without_runner_entries(
                r"C:\Runner\Data\bin;C:\Runner\Data\missions\id\shims\coder\bin;C:\Tools",
                Path::new(r"c:\runner\data"),
                runner_core::command_install::PathStyle::Windows,
            ),
            r"C:\Tools"
        );
    }

    #[cfg(unix)]
    #[test]
    fn command_status_ignores_runner_sidecar_and_shim_before_the_owned_link() {
        use std::os::unix::fs::{symlink, PermissionsExt as _};

        let temp = tempfile::tempdir().unwrap();
        let app_data = temp.path().join("app-data");
        let sidecar = app_data.join("bin/runner");
        let shim_dir = app_data.join("missions/mission/shims/coder/bin");
        let shim = shim_dir.join("runner");
        let home = temp.path().join("home");
        let local_bin = home.join(".local/bin");
        let link = local_bin.join("runner");
        for executable in [&sidecar, &shim] {
            std::fs::create_dir_all(executable.parent().unwrap()).unwrap();
            std::fs::write(executable, "runner").unwrap();
            std::fs::set_permissions(executable, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        std::fs::create_dir_all(&local_bin).unwrap();
        symlink(&sidecar, &link).unwrap();
        let path = format!(
            "{}:{}:{}",
            shim_dir.display(),
            sidecar.parent().unwrap().display(),
            local_bin.display()
        );

        let installed = command_install_status_with_path(&home, &sidecar, &app_data, false, &path);
        assert_eq!(installed["state"], "installed");

        let foreign_dir = temp.path().join("foreign-bin");
        let foreign = foreign_dir.join("runner");
        std::fs::create_dir_all(&foreign_dir).unwrap();
        std::fs::write(&foreign, "foreign").unwrap();
        std::fs::set_permissions(&foreign, std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = format!("{}:{path}", foreign_dir.display());
        let shadowed = command_install_status_with_path(&home, &sidecar, &app_data, false, &path);
        assert_eq!(shadowed["state"], "shadowed");
        assert_eq!(shadowed["shadowed_by"], foreign.display().to_string());
    }

    #[tokio::test]
    async fn nonexistent_full_mission_id_is_an_unresolvable_reference() {
        let client = FailingMissionGetClient { code: 1 };
        let error = resolve_mission(&client, "01MISSING00000000000000000")
            .await
            .unwrap_err();
        assert_eq!(error.code, 2);
        assert!(error.message.contains("mission not found"));
    }

    #[tokio::test]
    async fn full_mission_validation_preserves_not_running_exit_three() {
        let client = FailingMissionGetClient { code: 3 };
        let error = resolve_mission(&client, "01MISSING00000000000000000")
            .await
            .unwrap_err();
        assert_eq!(error.code, 3);
    }

    #[tokio::test]
    async fn project_delete_adds_the_resolved_id_for_quiet_output_only() {
        let cli = Cli::try_parse_from(["runner", "project", "delete", "Runner"]).unwrap();
        let response = run_connected(&RecordingClient::default(), &cli, &BusContext::OffBus)
            .await
            .unwrap();
        assert_eq!(response.value["id"], "project-id");
        assert_eq!(response.raw_json, r#"{"id":"changed-id"}"#);
    }

    #[tokio::test]
    async fn mission_lifecycle_uses_real_shapes_and_adds_the_crew_name() {
        const MISSION_ID: &str = "01M00000000000000000000000";
        let cases = [
            (
                "mission_stop",
                json!({"id": MISSION_ID, "title": "CLI", "status": "running", "crew_id": "crew-id"}),
                None,
                "stopped",
            ),
            (
                "mission_resume",
                json!({"mission_id": MISSION_ID, "resumed_session_ids": ["session-id"], "sessions": []}),
                Some(
                    json!({"id": MISSION_ID, "title": "CLI", "status": "running", "crew_id": "crew-id"}),
                ),
                "running",
            ),
            (
                "mission_archive",
                json!({"id": MISSION_ID, "title": "CLI", "status": "completed", "crew_id": "crew-id"}),
                None,
                "completed",
            ),
        ];
        for (tool, tool_value, fetched_mission, expected_status) in cases {
            let mut calls = vec![
                (
                    "mission_list",
                    Ok(json!([{"id": MISSION_ID, "title": "CLI"}])),
                ),
                (tool, Ok(tool_value.clone())),
            ];
            if let Some(mission) = fetched_mission {
                calls.push(("mission_get", Ok(mission)));
            }
            calls.push(("crew_list", Ok(json!([{"id": "crew-id", "name": "Peer"}]))));
            let client = SequenceClient::new(calls);
            let response = mission_lifecycle(&client, tool, Some("01M"), &BusContext::OffBus)
                .await
                .unwrap();
            assert_eq!(response.value["crew_name"], "Peer");
            assert_eq!(response.value["status"], expected_status);
            assert_eq!(
                response.raw_json,
                serde_json::to_string(&tool_value).unwrap()
            );
        }
    }

    #[tokio::test]
    async fn every_remote_command_builds_the_expected_tool_calls_without_a_socket() {
        const MISSION_ID: &str = "01M00000000000000000000000";
        const ARCHIVED_ID: &str = "01A00000000000000000000000";
        const SESSION_ID: &str = "01S00000000000000000000000";
        const MISSION_SESSION_ID: &str = "01X00000000000000000000000";
        let cwd = absolute_path(Path::new(".")).unwrap();
        let nested_cwd = absolute_path(Path::new("./x/../y")).unwrap();
        let cases = vec![
            (vec!["project", "list"], vec!["project_list"], json!({})),
            (
                vec!["project", "show", "Runner"],
                vec!["project_list", "project_get"],
                json!({"id": "project-id"}),
            ),
            (
                vec!["project", "create", "New"],
                vec!["project_create"],
                json!({"name": "New", "cwd": cwd}),
            ),
            (
                vec!["project", "rename", "Runner", "New"],
                vec!["project_list", "project_rename"],
                json!({"id": "project-id", "name": "New"}),
            ),
            (
                vec!["project", "delete", "Runner", "--force"],
                vec!["project_list", "project_delete"],
                json!({"id": "project-id", "force": true}),
            ),
            (vec!["role", "list"], vec!["role_list"], json!({})),
            (
                vec!["role", "show", "coder"],
                vec!["role_list", "role_get_by_handle"],
                json!({"handle": "coder"}),
            ),
            (
                vec!["role", "create", "coder", "--runtime", "codex"],
                vec!["role_create"],
                json!({
                    "handle": "coder",
                    "display_name": "coder",
                    "runtime": "codex",
                    "command": "codex",
                    "args": [],
                    "env": {},
                }),
            ),
            (
                vec!["role", "update", "coder", "--model", "gpt"],
                vec!["role_list", "role_update"],
                json!({"id": "role-id", "input": {"model": "gpt"}}),
            ),
            (
                vec!["role", "update", "coder", "--model", ""],
                vec!["role_list", "role_update"],
                json!({"id": "role-id", "input": {"model": ""}}),
            ),
            (
                vec!["role", "delete", "coder"],
                vec!["role_list", "role_delete"],
                json!({"id": "role-id"}),
            ),
            (vec!["crew", "list"], vec!["crew_list"], json!({})),
            (
                vec!["crew", "show", "Peer"],
                vec!["crew_list", "crew_get", "slot_list"],
                json!({"crew_id": "crew-id"}),
            ),
            (
                vec!["crew", "create", "Peer"],
                vec!["crew_create"],
                json!({"name": "Peer"}),
            ),
            (
                vec!["crew", "update", "Peer", "--purpose", "ship"],
                vec!["crew_list", "crew_update"],
                json!({"id": "crew-id", "input": {"purpose": "ship"}}),
            ),
            (
                vec!["crew", "update", "Peer", "--purpose", ""],
                vec!["crew_list", "crew_update"],
                json!({"id": "crew-id", "input": {"purpose": ""}}),
            ),
            (
                vec!["crew", "delete", "Peer"],
                vec!["crew_list", "crew_delete"],
                json!({"id": "crew-id"}),
            ),
            (
                vec![
                    "crew", "add", "Peer", "coder", "--as", "impl", "--effort", "high",
                ],
                vec!["crew_list", "role_list", "slot_create", "slot_update"],
                json!({"slot_id": "slot-new", "input": {"effort_override": "high"}}),
            ),
            (
                vec!["crew", "set", "Peer", "impl", "--model", "gpt"],
                vec!["crew_list", "slot_list", "slot_update"],
                json!({"slot_id": "slot-impl", "input": {"model_override": "gpt"}}),
            ),
            (
                vec!["crew", "remove", "Peer", "impl"],
                vec!["crew_list", "slot_list", "slot_delete"],
                json!({"slot_id": "slot-impl"}),
            ),
            (
                vec!["crew", "lead", "Peer", "lead"],
                vec!["crew_list", "slot_list", "slot_set_lead"],
                json!({"slot_id": "slot-lead"}),
            ),
            (
                vec!["crew", "order", "Peer", "lead", "impl"],
                vec!["crew_list", "slot_list", "slot_reorder"],
                json!({
                    "crew_id": "crew-id",
                    "ordered_slot_ids": ["slot-lead", "slot-impl"]
                }),
            ),
            (
                vec![
                    "mission", "start", "--crew", "Peer", "--title", "CLI", "--cwd", "./x/../y",
                ],
                vec!["crew_list", "mission_start"],
                json!({
                    "crew_id": "crew-id",
                    "project_id": null,
                    "title": "CLI",
                    "goal_override": null,
                    "cwd": nested_cwd,
                }),
            ),
            (
                vec!["mission", "list", "--crew", "Peer"],
                vec!["crew_list", "mission_list_summary"],
                json!({"crew_id": "crew-id"}),
            ),
            (
                vec!["mission", "show", "01M"],
                vec!["mission_list", "mission_status"],
                json!({"id": MISSION_ID}),
            ),
            (
                vec!["mission", "show", ARCHIVED_ID],
                vec!["mission_list", "mission_get", "mission_status"],
                json!({"id": ARCHIVED_ID}),
            ),
            (
                vec![
                    "mission",
                    "start",
                    "--crew",
                    "Peer",
                    "--goal",
                    "Ship it",
                    "--title",
                    "CLI",
                    "--project",
                    "Runner",
                ],
                vec!["crew_list", "project_list", "mission_start"],
                json!({
                    "crew_id": "crew-id",
                    "project_id": "project-id",
                    "title": "CLI",
                    "goal_override": "Ship it",
                    "cwd": null,
                }),
            ),
            (
                vec!["msg", "post", "hello", "--mission", ARCHIVED_ID],
                vec!["mission_list", "mission_get", "mission_post"],
                json!({
                    "mission_id": ARCHIVED_ID,
                    "text": "hello",
                    "to": null,
                }),
            ),
            (
                vec!["mission", "stop", "01M"],
                vec!["mission_list", "mission_stop"],
                json!({"id": MISSION_ID}),
            ),
            (
                vec!["mission", "resume", "01M"],
                vec!["mission_list", "mission_resume", "mission_get"],
                json!({"id": MISSION_ID}),
            ),
            (
                vec!["mission", "archive", "01M"],
                vec!["mission_list", "mission_archive"],
                json!({"id": MISSION_ID}),
            ),
            (
                vec!["mission", "unarchive", ARCHIVED_ID],
                vec!["mission_list", "mission_get", "mission_unarchive"],
                json!({"id": ARCHIVED_ID}),
            ),
            (
                vec!["mission", "rename", "01M", "New title"],
                vec!["mission_list", "mission_rename"],
                json!({"id": MISSION_ID, "title": "New title"}),
            ),
            (
                vec!["mission", "pin", "01M"],
                vec!["mission_list", "mission_pin"],
                json!({"id": MISSION_ID, "pinned": true}),
            ),
            (
                vec!["mission", "unpin", "01M"],
                vec!["mission_list", "mission_pin"],
                json!({"id": MISSION_ID, "pinned": false}),
            ),
            (
                vec!["mission", "move", "01M", "--unfile"],
                vec!["mission_list", "mission_set_project"],
                json!({"mission_id": MISSION_ID, "project_id": null}),
            ),
            (
                vec!["mission", "feed", "01M", "--since", "7", "--limit", "2"],
                vec!["mission_list", "mission_feed"],
                json!({
                    "mission_id": MISSION_ID,
                    "since_offset": 7,
                    "limit": 2,
                    "order": "newest_first",
                }),
            ),
            (
                vec!["mission", "answer", "01M", "question", "yes"],
                vec!["mission_list", "mission_signal"],
                json!({
                    "mission_id": MISSION_ID,
                    "signal_type": "human_response",
                    "payload": {"question_id": "question", "choice": "yes"},
                }),
            ),
            (
                vec!["chat", "start", "coder", "--project", "Runner"],
                vec!["role_list", "project_list", "session_start_direct"],
                json!({
                    "role_id": "role-id",
                    "runtime": null,
                    "model": null,
                    "effort": null,
                    "project_id": "project-id",
                    "cwd": null,
                }),
            ),
            (
                vec!["chat", "start", "--runtime", "codex", "--cwd", "."],
                vec!["session_start_direct"],
                json!({
                    "role_id": null,
                    "runtime": "codex",
                    "model": null,
                    "effort": null,
                    "project_id": null,
                    "cwd": cwd,
                }),
            ),
            (vec!["session", "list"], vec!["session_list"], json!({})),
            (
                vec!["session", "show", "01S"],
                vec!["session_list", "session_get"],
                json!({"session_id": SESSION_ID}),
            ),
            (
                vec!["session", "stop", "01S"],
                vec!["session_list", "session_stop"],
                json!({"session_id": SESSION_ID}),
            ),
            (
                vec!["session", "archive", "01S"],
                vec!["session_list", "session_archive"],
                json!({"session_id": SESSION_ID}),
            ),
            (
                vec!["session", "resume", "01S"],
                vec!["session_list", "session_resume"],
                json!({"session_id": SESSION_ID}),
            ),
            (
                vec!["session", "restart", "01S"],
                vec!["session_list", "session_restart"],
                json!({"session_id": SESSION_ID}),
            ),
            (
                vec!["session", "restart", MISSION_SESSION_ID],
                vec!["session_list", "session_restart"],
                json!({"session_id": MISSION_SESSION_ID}),
            ),
            (
                vec![
                    "msg",
                    "post",
                    "hello",
                    "--mission",
                    "01M",
                    "--to",
                    "lead",
                    "--as",
                    "coder",
                ],
                vec!["mission_list", "mission_post"],
                json!({
                    "mission_id": MISSION_ID,
                    "text": "hello",
                    "to": "lead",
                    "from": "coder",
                }),
            ),
            (
                vec![
                    "signal",
                    "ask_lead",
                    "--mission",
                    "01M",
                    "--as",
                    "coder",
                    "--payload",
                    "{}",
                ],
                vec!["mission_list", "mission_signal"],
                json!({
                    "mission_id": MISSION_ID,
                    "signal_type": "ask_lead",
                    "payload": {},
                    "from": "coder",
                }),
            ),
            (
                vec![
                    "ask",
                    "Question?",
                    "--context",
                    "Context",
                    "--mission",
                    "01M",
                    "--as",
                    "coder",
                ],
                vec!["mission_list", "mission_signal"],
                json!({
                    "mission_id": MISSION_ID,
                    "signal_type": "ask_lead",
                    "payload": {"question": "Question?", "context": "Context"},
                    "from": "coder",
                }),
            ),
            (
                vec![
                    "ask",
                    "--human",
                    "Ship?",
                    "--choices",
                    "yes,no",
                    "--mission",
                    "01M",
                    "--as",
                    "coder",
                ],
                vec!["mission_list", "mission_signal"],
                json!({
                    "mission_id": MISSION_ID,
                    "signal_type": "ask_human",
                    "payload": {"prompt": "Ship?", "choices": ["yes", "no"]},
                    "from": "coder",
                }),
            ),
            (
                vec!["call", "role_get", r#"{"id":"role-id"}"#],
                vec!["role_get"],
                json!({"id": "role-id"}),
            ),
        ];

        let mut reached_tools = std::collections::BTreeSet::new();
        for (args, expected_tools, expected_arguments) in cases {
            let argv = std::iter::once("runner")
                .chain(args.iter().copied())
                .collect::<Vec<_>>();
            let cli = Cli::try_parse_from(&argv).unwrap_or_else(|error| {
                panic!("failed to parse {argv:?}: {error}");
            });
            let context = BusContext::OffBus;
            validate_remote(&cli, &context).unwrap_or_else(|error| {
                panic!("failed to validate {argv:?}: {}", error.message);
            });
            let client = RecordingClient::default();
            run_connected(&client, &cli, &context)
                .await
                .unwrap_or_else(|error| panic!("failed to run {argv:?}: {}", error.message));
            let calls = client.calls.into_inner().unwrap();
            let tools = calls
                .iter()
                .map(|(tool, _)| tool.as_str())
                .collect::<Vec<_>>();
            reached_tools.extend(calls.iter().map(|(tool, _)| tool.clone()));
            assert_eq!(tools, expected_tools, "wrong tool sequence for {argv:?}");
            assert_eq!(
                calls.last().unwrap().1,
                expected_arguments,
                "wrong final arguments for {argv:?}"
            );
        }
        let registry_tools = runner_core::RUNNER_TOOL_NAMES
            .iter()
            .map(|tool| (*tool).to_owned())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(reached_tools, registry_tools);
    }

    #[test]
    fn every_command_leaf_parses_without_a_socket() {
        let cases: &[&[&str]] = &[
            &["status"],
            &["project", "list"],
            &["project", "show", "Runner"],
            &["project", "create", "Runner", "--path", "."],
            &["project", "rename", "Runner", "New"],
            &["project", "delete", "Runner", "--force"],
            &["role", "list"],
            &["role", "show", "coder"],
            &["role", "create", "coder", "--runtime", "codex"],
            &["role", "update", "coder", "--model", "gpt"],
            &["role", "delete", "coder"],
            &["crew", "list"],
            &["crew", "show", "Peer"],
            &["crew", "create", "Peer"],
            &["crew", "update", "Peer", "--purpose", "ship"],
            &["crew", "delete", "Peer"],
            &["crew", "add", "Peer", "coder", "--as", "impl"],
            &["crew", "set", "Peer", "impl", "--effort", "high"],
            &["crew", "remove", "Peer", "impl"],
            &["crew", "lead", "Peer", "impl"],
            &["crew", "order", "Peer", "lead", "impl"],
            &["mission", "list", "--crew", "Peer"],
            &["mission", "show", "01M"],
            &["mission", "start", "--crew", "Peer", "--goal", "ship"],
            &["mission", "stop", "01M"],
            &["mission", "resume", "01M"],
            &["mission", "archive", "01M"],
            &["mission", "unarchive", "01M"],
            &["mission", "rename", "01M", "Title"],
            &["mission", "pin", "01M"],
            &["mission", "unpin", "01M"],
            &["mission", "move", "01M", "--unfile"],
            &["mission", "feed", "01M", "--oldest-first"],
            &["mission", "answer", "01M", "question", "yes"],
            &["chat", "start", "coder"],
            &["chat", "start", "--runtime", "codex"],
            &["session", "list"],
            &["session", "show", "01S"],
            &["session", "stop", "01S"],
            &["session", "archive", "01S"],
            &["session", "resume", "01S"],
            &["session", "restart", "01S"],
            &["msg", "post", "hello", "--mission", "01M"],
            &["msg", "read"],
            &["signal", "ask_lead", "--mission", "01M"],
            &["ask", "question", "--mission", "01M", "--as", "coder"],
            &[
                "ask",
                "--human",
                "ship?",
                "--choices",
                "yes,no",
                "--mission",
                "01M",
                "--as",
                "coder",
            ],
            &["call", "mission_list", "{}"],
            &["help", "mission"],
        ];
        for args in cases {
            let argv = std::iter::once("runner")
                .chain(args.iter().copied())
                .collect::<Vec<_>>();
            assert!(
                Cli::try_parse_from(&argv).is_ok(),
                "failed to parse {argv:?}"
            );
        }
    }
}
