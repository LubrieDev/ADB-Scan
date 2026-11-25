//! Error type of the core. Its messages are shown in the interface, so they
//! go through the catalog (`Display` is written by hand instead of with
//! `thiserror`, whose messages are fixed at compile time).

use std::fmt;

use crate::t;

#[derive(Debug)]
pub enum Error {
    ToolNotFound { tool: String },
    Spawn { program: String, source: std::io::Error },
    Command { summary: String, code: Option<i32> },
    DeviceNotFound(String),
    Invalid(String),
    Guarded(String),
    Db(rusqlite::Error),
    Io(std::io::Error),
    Json(serde_json::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Error::ToolNotFound { tool } => t!("{} was not found. Install it or set its path in Preferences.", tool),
            Error::Spawn { program, source } => t!("Could not run {}: {}", program, source),
            Error::Command { summary, .. } => summary.clone(),
            Error::DeviceNotFound(serial) => t!("The device {} is not available", serial),
            Error::Invalid(what) => t!("Invalid value: {}", what),
            Error::Guarded(what) => t!("Operation blocked for safety: {}", what),
            Error::Db(e) => t!("Database: {}", e),
            Error::Io(e) => t!("I/O: {}", e),
            Error::Json(e) => format!("JSON: {e}"),
        };
        f.write_str(&text)
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Spawn { source, .. } => Some(source),
            Error::Db(e) => Some(e),
            Error::Io(e) => Some(e),
            Error::Json(e) => Some(e),
            _ => None,
        }
    }
}

impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Error::Db(e)
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Json(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;
