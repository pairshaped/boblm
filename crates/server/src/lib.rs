//! HTTP transport components. Authentication and application assembly belong to Server.
pub mod sse;

pub mod app;

pub mod mcp;
pub mod schedule;
pub mod secrets;
pub mod uploads;
pub mod web;

pub mod usage_report;

pub mod role_spawn;

pub mod agent_images;
mod assets;
pub mod config;

pub mod pi;

mod directories;
mod idle_nudge;

pub mod questions;

pub mod notifications;

mod browser_reload;

mod subagent_history;

mod subagent_lifetime;

mod subagent_report;

mod subagent_wake;

pub mod persistent;

mod claude_account;
