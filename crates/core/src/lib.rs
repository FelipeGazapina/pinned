//! Everything pinned does without a window: hosting `claude` in PTYs, answering Claude Code's
//! callbacks, and the connectors. The desktop app and the headless server both build on it.

pub mod agent;
pub mod connectors;
pub mod evidence;
pub mod history;
pub mod hooks;
pub mod linear;
pub mod plugin;
pub mod projects;
pub mod pty;
pub mod reminders;
pub mod sessions;
pub mod slack;
