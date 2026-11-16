pub mod github;
#[cfg(any(test, debug_assertions))]
pub mod fake_github;
mod hub;

pub use hub::{prepare, Config, Reports, DAILY_LIMIT, NOT_AVAILABLE, TOKEN_VARIABLE};
