//! Tools for the voice assistant: the registry (the enforcement point), plugins, MCP,
//! terminal jobs and the built-in typed tools. Decisions live in `assistant-policy`.

pub mod agent;
pub mod jobs;
pub mod mcp;
pub mod plugins;
pub mod registry;
pub mod typed;
pub mod verify;
