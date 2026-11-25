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
    /// Reachable from the internet, per its latest advert.
    pub reachable: bool,
    /// Data can flow to it now (a direct link or a live connection).
    pub connected: bool,
}

fn identity_file(dir: &Path) -> PathBuf {
    dir.join("identity.nsec")
}

/// Run `f` on the device of `dir`: the app's running client, or one
/// started for this call.
fn with_device<T>(dir: &Path, f: impl FnOnce(&Client) -> Result<T>) -> Result<T> {
    with_device_joining(dir, Vec::new(), f)
}

/// As [`with_device`], a device started here using only `join_relays` (a
/// pairing code's) until its join ends, so nothing of it (not even its key
/// package) reaches the public relays.
fn with_device_joining<T>(dir: &Path, join_relays: Vec<String>, f: impl FnOnce(&Client) -> Result<T>) -> Result<T> {
    if let Some(c) = Client::running(dir) {
        return f(&c);
    }
    let c = Client::start(ClientConfig {
        identity: IdentitySource::File(identity_file(dir)),
        join_relays,
        mesh_dir: Some(dir.to_path_buf()),
        ..ClientConfig::default()
    })?;
    f(&c)
}

/// The members of a running client.
pub fn members_of(c: &Client) -> Vec<MeshMember> {
    c.members()
        .into_iter()
        .map(|m| MeshMember { npub: m.npub, name: m.name, last_seen_secs: m.last_seen_secs, reachable: m.reachable, connected: m.connected })
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
    with_device_joining(dir, noscat_client::parse_pairing_relays(owner_npub), |c| {
        c.join(owner_npub, Duration::from_secs(timeout_secs)).map_err(|e| {
            let text = e.to_string();
            if text.contains("timed out") { anyhow!("no invitation arrived in time") } else { anyhow!(text) }
        })?;
        Ok(members_of(c))
    })
}

/// Leave the mesh: ask the other members to remove this device and wait
/// up to `timeout_secs` for one to do it. With no member online in time it
/// forgets the mesh locally anyway; the request stays on the relays and the
/// next member to come online completes it. A device that is in no mesh
/// has nothing to leave.
pub fn leave(dir: &Path, timeout_secs: u64) -> Result<()> {
    let res = with_device(dir, |c| match c.leave(Duration::from_secs(timeout_secs)) {
        Ok(()) => Ok(()),
        Err(e) if e.to_string().contains("not in a mesh") => Ok(()),
        Err(e) => Err(anyhow!(e.to_string())),
    });
    match res {
        // Sealed files whose key is gone: this device can never open, so
        // never leave; Forget deletes it anyway (members still list it
        // until one removes it).
        Err(e) if e.to_string().contains("is encrypted") => Ok(()),
        other => other,
    }
}

/// Delete the device directory (identity and mesh state) once no client
/// runs on it, so the next join starts as a new device. Waits up to five
/// seconds for a dropped client to finish stopping.
pub fn wipe(dir: &Path) -> Result<()> {
    for _ in 0..50 {
        if Client::running(dir).is_none() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    if Client::running(dir).is_some() {
        return Err(anyhow!("the device is still running"));
    }
    match std::fs::remove_dir_all(dir) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
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
