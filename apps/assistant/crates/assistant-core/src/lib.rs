//! Shared types for Nyx: config, state machine, events, timing.

pub mod apps;
pub mod config;
pub mod event;
pub mod paths;
pub mod state;
pub mod timing;

pub use apps::{AppLaunch, AppsFile};
pub use config::Config;
pub use event::Event;
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
    Toml(#[from] toml::de::Error),
}

impl Error {
    pub fn msg(m: impl Into<String>) -> Self {
        Self::Msg(m.into())
    }
}

pub type Result<T> = std::result::Result<T, Error>;
