//! XDG data locations and persisted settings. Everything the app keeps on
//! disk lives under the platform data dir (`~/.local/share/noscat-desktop`
//! on Linux), deliberately separate from the CLI's so the desktop app is
//! its own device in the mesh. The one identity is the device dir's
//! `identity.nsec`: join screen and client both use it.
//!
//! noscat 0.5 starts fresh: its device lives in `device/` and its member
//! cache in `members.json`, so 0.4's `mesh/` and `roster.json` are left
//! alone and never read.



use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub fn data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("noscat-desktop")
}

pub fn mesh_dir() -> PathBuf {
    data_dir().join("device")
}

pub fn roster_file() -> PathBuf {
    data_dir().join("members.json")
}

/// Chat history, per conversation.
pub fn chats_file() -> PathBuf {
    data_dir().join("chats.json")
}

/// Transfer history: the Sent list and who sent each received file.
pub fn transfers_file() -> PathBuf {
    data_dir().join("transfers.json")
}

pub fn settings_file() -> PathBuf {
    data_dir().join("settings.json")
}

/// Where inbound files land: `~/Downloads/noscat`, or an inbox under the
/// data dir when the system download dir is unavailable.
pub fn inbox_dir() -> PathBuf {
    let base = dirs::download_dir().unwrap_or_else(|| data_dir().join("inbox"));
    base.join("noscat")
}

/// A sensible default device name: the hostname, like the CLI uses.
fn default_device_name() -> String {
    hostname()
        .unwrap_or_else(|| "desktop".to_string())
}

fn hostname() -> Option<String> {
    std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Which appearance the app renders in. Default follows the desktop's
/// system theme.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThemePref {
    #[default]
    #[serde(rename = "system")]
    System,
    #[serde(rename = "light")]
    Light,
    #[serde(rename = "dark")]
    Dark,
}

/// User-editable settings, persisted to `settings.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    /// Friendly name published in the advert; peers display it.
    pub device_name: String,
    /// Appearance: follow the system, or force light or dark.
    pub theme: ThemePref,
    /// This device is reachable from the internet (UDP 41641 forwarded to
    /// it, or a public address): the mesh's meeting point for devices on
    /// other networks, like `noscat up --reachable`.
    #[serde(default)]
    pub reachable: bool,
    /// The mesh's name as last read from the running client, so the
    /// sidebar can name the mesh offline and at startup.
    #[serde(default)]
    pub mesh_name: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            device_name: default_device_name(),
            theme: ThemePref::System,
            reachable: false,
            mesh_name: None,
        }
    }
}

impl Settings {
    pub fn load() -> Self {
        fs::read_to_string(settings_file())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn store(&self) -> std::io::Result<()> {
        let dir = data_dir();
        fs::create_dir_all(&dir)?;
        fs::write(settings_file(), serde_json::to_string_pretty(self)?)
    }
}
