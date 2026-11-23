//! XDG data locations and persisted settings. Everything the app keeps on
//! disk lives under the platform data dir (`~/.local/share/noscat-desktop`
//! on Linux), deliberately separate from the CLI's `~/.noscat` so the
//! desktop app is its own device in the mesh.

#![allow(dead_code)]

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub fn data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("noscat-desktop")
}

pub fn identity_file() -> PathBuf {
    data_dir().join("identity.hex")
}

pub fn mesh_dir() -> PathBuf {
    data_dir().join("mesh")
}

pub fn roster_file() -> PathBuf {
    data_dir().join("roster.json")
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

/// User-editable settings, persisted to `settings.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    /// Friendly name published in the advert; peers display it.
    pub device_name: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            device_name: default_device_name(),
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
