//! Mesh helpers: the Rust-to-Rust counterpart of the `mesh_*` functions in
//! the upstream `noscat-ffi` crate. One device runs per directory: these use
//! the app's running client for the mesh directory when there is one, and
//! otherwise run the device just for the call.
//!
//! Every function here blocks (relay round trips, seconds). Call only from
//! the background executor.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Result, anyhow};
use noscat_client::{Client, ClientConfig, IdentitySource};

/// A member device of the mesh: how the device list shows it.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MeshMember {
    /// The device's npub (bech32), the target for sends and forwards.
    pub npub: String,
    /// The device's name from its advert.
    pub name: Option<String>,
    /// Seconds since its last advert; None = not seen since this device
    /// started.
    pub last_seen_secs: Option<u64>,
}

fn identity_file(dir: &Path) -> PathBuf {
    dir.join("identity.nsec")
}

/// Run `f` on the device of `dir`: the app's running client, or one
/// started for this call.
fn with_device<T>(dir: &Path, f: impl FnOnce(&Client) -> Result<T>) -> Result<T> {
    if let Some(c) = Client::running(dir) {
        return f(&c);
    }
    let c = Client::start(ClientConfig {
        identity: IdentitySource::File(identity_file(dir)),
        mesh_dir: Some(dir.to_path_buf()),
        ..ClientConfig::default()
    })?;
    f(&c)
}

/// The members of a running client.
pub fn members_of(c: &Client) -> Vec<MeshMember> {
    c.members()
        .into_iter()
        .map(|m| MeshMember { npub: m.npub, name: m.name, last_seen_secs: m.last_seen_secs })
        .collect()
}

/// Prepare to join: this device's npub, the one the inviting device adds.
/// The join request itself goes out in [`join`]. `owner_npub` is unused.
pub fn prepare(dir: &Path, owner_npub: Option<&str>) -> Result<String> {
    let _ = owner_npub;
    device_npub(dir)
}

/// Wait up to `timeout_secs` for an invitation from `owner_npub` (a bare
/// npub or the QR's `noscat:` URI) and join. The inviting device is asked
/// to invite this one. Invitations from anyone else are ignored: the QR is
/// the trust anchor. Returns the members.
pub fn join(dir: &Path, timeout_secs: u64, owner_npub: &str) -> Result<Vec<MeshMember>> {
    with_device(dir, |c| {
        c.join(owner_npub, Duration::from_secs(timeout_secs)).map_err(|e| {
            let text = e.to_string();
            if text.contains("timed out") { anyhow!("no invitation arrived in time") } else { anyhow!(text) }
        })?;
        Ok(members_of(c))
    })
}

/// This device's npub. There is exactly one identity: the mesh dir's
/// `identity.nsec`, which the client runs on and the join screen shows.
/// Local only (creates the key on first run), so safe before any connect.
pub fn device_npub(dir: &Path) -> Result<String> {
    std::fs::create_dir_all(dir)?;
    if let Some(c) = Client::running(dir) {
        return Ok(c.npub().to_string());
    }
    Ok(noscat_client::identity_npub(&IdentitySource::File(identity_file(dir)))?)
}
