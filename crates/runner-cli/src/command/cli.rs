use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};
use serde_json::{json, Value};

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
    pub(super) json: bool,
    /// Print only result IDs.
    #[arg(short = 'q', global = true, conflicts_with = "json")]
    pub(super) quiet: bool,
    #[command(subcommand)]
    pub(super) command: Command,
}

#[derive(Subcommand, Debug)]
pub(super) enum Command {
    /// Connect this computer to your remote access account.
    Remote {
        #[command(subcommand)]
        command: crate::remote::RemoteCommand,
    },
    #[command(hide = true)]
    Hook {
        #[command(subcommand)]
        command: runner_cli::hook::Command,
    },
    /// Inspect or stop the local daemon.
    Daemon {
        #[command(subcommand)]
        command: DaemonCommand,
    },
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
    /// Ask the person at the app.
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
pub(super) enum DaemonCommand {
    Status,
    Stop,
}

#[derive(Subcommand, Debug)]
pub(super) enum ProjectCommand {
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
pub(super) struct RoleFields {
    /// Display name.
    #[arg(long)]
    pub(super) name: Option<String>,
    /// Agent runtime registry name.
    #[arg(long)]
    pub(super) runtime: Option<String>,
    /// Model override; pass an empty value to clear it.
    #[arg(long)]
    pub(super) model: Option<String>,
    /// Reasoning effort; pass an empty value to clear it.
    #[arg(long)]
    pub(super) effort: Option<String>,
    /// Codex Speed: inherit, standard, or fast.
    #[arg(long)]
    pub(super) speed: Option<SpeedArg>,
    /// Permission mode: default, accept_edits, auto, or bypass.
    #[arg(long)]
    pub(super) permission: Option<String>,
    /// Inline system prompt; pass an empty value to clear it.
    #[arg(long, conflicts_with = "prompt_file")]
    pub(super) prompt: Option<String>,
    /// Read the system prompt from a file, or - for stdin.
    #[arg(long, conflicts_with = "prompt")]
    pub(super) prompt_file: Option<PathBuf>,
    /// Set one KEY=VALUE environment entry; repeat as needed.
    #[arg(long = "env")]
    pub(super) env: Vec<String>,
    /// Working directory; pass an empty value to clear it.
    #[arg(long)]
    pub(super) cwd: Option<String>,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(super) enum SpeedArg {
    Inherit,
    Standard,
    Fast,
}

impl SpeedArg {
    pub(super) fn value(self) -> Value {
        match self {
            Self::Inherit => Value::Null,
            Self::Standard => json!("standard"),
            Self::Fast => json!("fast"),
        }
    }
}

#[derive(Subcommand, Debug)]
pub(super) enum RoleCommand {
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
pub(super) struct CrewCreateFields {
    /// Read crew conventions from a file, or - for stdin.
    #[arg(long)]
    pub(super) conventions_file: Option<PathBuf>,
}

#[derive(Args, Debug, Default)]
pub(super) struct CrewFields {
    /// New crew name.
    #[arg(long)]
    pub(super) name: Option<String>,
    /// Read crew conventions from a file, or - for stdin.
    #[arg(long)]
    pub(super) conventions_file: Option<PathBuf>,
}

#[derive(Subcommand, Debug)]
pub(super) enum CrewCommand {
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
        /// Codex Speed override: inherit, standard, or fast.
        #[arg(long)]
        speed: Option<SpeedArg>,
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
        /// Codex Speed override: inherit, standard, or fast.
        #[arg(long)]
        speed: Option<SpeedArg>,
    },
    /// Remove a slot from a crew.
    Remove { crew: String, handle: String },
    /// Reorder every crew slot by handle.
    Order {
        crew: String,
        #[arg(required = true, num_args = 1..)]
        handles: Vec<String>,
    },
}

#[derive(Subcommand, Debug)]
pub(super) enum MissionCommand {
    /// List active missions.
    List {
        /// Filter by crew ID or exact name.
        #[arg(long, conflicts_with = "linked_here")]
        crew: Option<String>,
        /// List the missions linked to this chat.
        #[arg(long)]
        linked_here: bool,
    },
    /// Show an operational mission snapshot.
    Show { mission: Option<String> },
    /// Start a mission.
    Start {
        /// Crew ID or exact name.
        #[arg(long)]
        crew: Option<String>,
        /// Mission goal text.
        #[arg(long, conflicts_with = "goal_file")]
        goal: Option<String>,
        /// Read the mission goal from a file, or - for stdin.
        #[arg(long, conflicts_with = "goal")]
        goal_file: Option<PathBuf>,
        /// Mission title.
        #[arg(long)]
        title: String,
        /// Project ID or exact name whose directory the mission uses.
        #[arg(long, conflicts_with = "cwd")]
        project: Option<String>,
        /// Mission directory; defaults to the current directory.
        #[arg(long, conflicts_with = "project")]
        cwd: Option<PathBuf>,
        /// Link the mission to this chat without typing reports into it.
        #[arg(long)]
        no_report: bool,
    },
    /// Add a configured worker role to a running mission (person only).
    AddRole {
        mission: String,
        role: String,
        #[arg(long)]
        handle: Option<String>,
        #[arg(long)]
        runtime: Option<String>,
        #[arg(long)]
        model: Option<String>,
        #[arg(long)]
        effort: Option<String>,
        #[arg(long)]
        speed: Option<SpeedArg>,
        #[arg(long, conflicts_with = "task_file")]
        task: Option<String>,
        #[arg(long, conflicts_with = "task")]
        task_file: Option<PathBuf>,
    },
    /// Remove a worker from a running mission (person only).
    RemoveRole { mission: String, handle: String },
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
    /// Link a mission to a chat, which Runner then tells when it needs you, fails or settles.
    Link {
        mission: String,
        /// Direct chat session ID or unique prefix; defaults to this chat.
        #[arg(long)]
        chat: Option<String>,
        /// Link without typing reports into the chat.
        #[arg(long)]
        no_report: bool,
    },
    /// Remove a mission's link to its chat.
    Unlink { mission: String },
    /// Print a window from the mission event feed.
    Feed {
        mission: Option<String>,
        /// Follow pushed events until the mission ends or all its sessions exit.
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
        /// A condition the choice does not carry, delivered with the answer.
        #[arg(long, conflicts_with = "note_file")]
        note: Option<String>,
        /// Read the note from a file, or from stdin with `-`.
        #[arg(long, conflicts_with = "note")]
        note_file: Option<PathBuf>,
    },
}

#[derive(Subcommand, Debug)]
pub(super) enum ChatCommand {
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
        /// Codex Speed override: inherit, standard, or fast.
        #[arg(long)]
        speed: Option<SpeedArg>,
        /// Project ID or exact name whose directory the chat uses.
        #[arg(long, conflicts_with = "cwd")]
        project: Option<String>,
        /// Chat directory; defaults to the current directory.
        #[arg(long, conflicts_with = "project")]
        cwd: Option<PathBuf>,
    },
}

#[derive(Subcommand, Debug)]
pub(super) enum SessionCommand {
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
pub(super) enum MsgCommand {
    /// Post a mission message.
    Post {
        text: String,
        /// Address a roster handle; repeat for several recipients.
        #[arg(long)]
        to: Vec<String>,
        /// Mission ID or unique prefix; required outside a mission.
        #[arg(long)]
        mission: Option<String>,
        /// Attribute the message to this roster handle.
        #[arg(long = "as")]
        from: Option<String>,
    },
    /// Read new inbox messages; --all shows crew history since joining.
    Read {
        /// Show all crew messages since joining.
        #[arg(long)]
        all: bool,
        /// Return messages after this event ID without marking them read.
        #[arg(long)]
        since: Option<String>,
        /// Filter by sender handle without marking messages read.
        #[arg(long)]
        from: Option<String>,
        /// Mission reference; only the caller's mission is supported.
        #[arg(long)]
        mission: Option<String>,
    },
}

#[derive(Args, Debug)]
pub(super) struct AskArgs {
    /// Ask the person at the app.
    pub(super) question: Option<String>,
    /// Ask the person at the app.
    #[arg(long, conflicts_with = "question")]
    pub(super) human: Option<String>,
    /// Comma-separated choices for the person.
    #[arg(long)]
    pub(super) choices: Option<String>,
    /// Mission ID or unique prefix; required outside a mission.
    #[arg(long)]
    pub(super) mission: Option<String>,
    /// Attribute the question to this roster handle.
    #[arg(long = "as")]
    pub(super) from: Option<String>,
}
