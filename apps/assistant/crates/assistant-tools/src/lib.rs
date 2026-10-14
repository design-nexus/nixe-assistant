//! Tools for the voice assistant: the registry (the enforcement point), plugins, MCP,
//! terminal jobs and the built-in typed tools. Decisions live in `assistant-policy`.

pub mod admin;
pub mod agent;
pub mod fs;
pub mod jobs;
pub mod mcp;
pub mod plugins;
pub mod registry;
pub mod relevance;
pub mod sandbox;
pub mod system;
pub mod typed;
pub mod verify;
