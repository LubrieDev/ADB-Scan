//! ADB Scan core.
//!
//! It does not depend on any graphical library: everything here is tested with
//! real outputs captured from a device, without a phone connected.

pub mod adb;
pub mod apk;
pub mod automation;
pub mod app_inspect;
pub mod basic;
pub mod changes;
pub mod battery_usage;
pub mod cleanup;
pub mod controls;
pub mod db;
pub mod debloat;
pub mod devtools;
pub mod error;
pub mod exec;
pub mod fastboot;
pub mod health_report;
pub mod format;
pub mod i18n;
pub mod identity;
pub mod link;
pub mod live;
pub mod migration;
pub mod models;
pub mod monitor;
pub mod mtp;
pub mod notifications;
pub mod parsers;
pub mod permissions;
pub mod paths;
pub mod photos;
pub mod platform;
pub mod privacy;
pub mod probe;
pub mod remote;
pub mod schedule;
pub mod scrcpy;
pub mod security;
pub mod settings;
pub mod storage;
pub mod tools_setup;
pub mod tracker;
pub mod updater;
pub mod usb;
pub mod wireless;

pub use error::{Error, Result};
