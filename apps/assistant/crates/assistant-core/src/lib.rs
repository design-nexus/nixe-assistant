//! Shared types for the voice assistant: config, state machine, events, timing.

pub mod apps;
pub mod commands;
pub mod config;
pub mod event;
pub mod location;
pub mod paths;
pub mod persona;
pub mod state;
pub mod timing;
pub mod weather;

pub use apps::{AppLaunch, AppsFile};
pub use commands::{CommandPhrase, CommandsFile, WorkspaceCommands};
pub use config::Config;
pub use event::Event;
pub use persona::{sync_plugin_manifest, Persona};
pub use state::{AssistantState, StateMachine};
pub use timing::TurnTiming;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("{0}")]
    Msg(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    TomlDe(#[from] toml::de::Error),
    #[error(transparent)]
    TomlSer(#[from] toml::ser::Error),
}

impl Error {
    pub fn msg(m: impl Into<String>) -> Self {
        Self::Msg(m.into())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

pub mod diagnostics;
pub mod routing;
