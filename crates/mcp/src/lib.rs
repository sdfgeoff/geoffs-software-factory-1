mod client;
mod server;
pub mod tool;
mod tools;

pub use client::{BoardClient, McpError};
pub use server::run;
