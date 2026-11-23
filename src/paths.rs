//! XDG data locations and persisted settings. Everything the app keeps on
//! disk lives under the platform data dir (`~/.local/share/noscat-desktop`
//! on Linux), deliberately separate from the CLI's `~/.noscat` so the
//! desktop app is its own device in the mesh. The one identity is the mesh
//! dir's `identity.nsec`: join screen, key package, and client all use it.



use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub fn data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("noscat-desktop")
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
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            device_name: default_device_name(),
            theme: ThemePref::System,
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
