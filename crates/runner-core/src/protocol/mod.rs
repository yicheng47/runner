pub mod api;
pub mod client;
pub mod command;
pub mod crew;
pub mod discovery;
pub mod hook;
pub mod managed;
pub mod mission;
pub mod mod_types;
pub mod model;
pub mod node;
pub mod permissions;
pub mod project;
pub mod role;
pub mod runtime;
pub mod session;
pub mod session_title;
pub mod skills;
pub mod slot;
pub mod socket;
pub mod status;
pub mod terminal;
pub mod usage;
pub mod window;
pub mod wire;
pub use api::{Request, Response};
pub use client::*;
pub use crew::*;
pub use discovery::*;
pub use hook::HookReport;
pub use mission::*;
pub use mod_types::*;
pub use model::*;
pub use node::*;
pub use permissions::*;
pub use project::*;
pub use role::*;
pub use runtime::*;
pub use session::*;
pub use slot::*;
pub use status::*;
pub use usage::*;
pub use window::*;

pub fn double_option<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    <Option<T> as serde::Deserialize>::deserialize(deserializer).map(Some)
}

pub mod runtime_metadata;
pub mod runtime_options;

pub mod mcp;
pub use mcp::*;

pub use discovery::DiscoverySnapshot;

pub mod agent_skill;

pub mod runtime_paths;

#[cfg(test)]
mod tests;
