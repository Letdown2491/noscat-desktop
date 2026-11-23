//! Mesh helpers: the Rust-to-Rust counterpart of the glue in the upstream
//! `noscat-ffi` crate (mesh_prepare/mesh_join/mesh_sync/mesh_members). These
//! compose `noscat_core::mesh::Device` with relay fetches; they are
//! app-layer composition, not protocol code.
//!
//! Every function here blocks (relay round trips, seconds). Call only from
//! the background executor.

#![allow(dead_code)]

use std::path::Path;

use anyhow::{Result, anyhow};
use nostr::prelude::PublicKey;
use noscat_core::relay::DEFAULT_RELAYS;

fn relays() -> Vec<String> {
    DEFAULT_RELAYS.iter().map(|s| s.to_string()).collect()
}

/// A member device of a mesh group: how the device list shows it.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MeshMember {
    /// The device's npub (bech32), the dial target for tunnels.
    pub npub: String,
    /// The same key in hex, the form the client's presence watcher speaks.
    pub hex: String,
    /// True for the device that owns (created) the mesh.
    pub is_owner: bool,
    /// The device's friendly name from its advert, when it is serving and
    /// published one.
    pub name: Option<String>,
    /// Seconds since the device's advert was last seen. Online = within the
    /// advert TTL; absent = no advert on the relays, treat as offline.
    pub last_seen_secs: Option<u64>,
}

fn members_of(device: noscat_core::mesh::Device) -> Result<Vec<MeshMember>> {
    use nostr::nips::nip19::ToBech32 as _;
    Ok(device
        .list_members()?
        .into_iter()
        .map(|m| {
            Ok(MeshMember {
                // list_members returns x-only hex; the UI and dial targets
                // speak bech32 npubs, like the FFI crate does.
                npub: PublicKey::from_hex(&m.npub)?.to_bech32()?,
                hex: m.npub,
                is_owner: m.is_owner,
                name: None,
                last_seen_secs: None,
            })
        })
        .collect::<Result<Vec<_>>>()?)
}

/// Enrich the bare member list with advert names and last-seen ages,
/// fetched concurrently. A member whose advert fetch fails keeps
/// `name: None` and no last-seen: the caller ages its previous values
/// forward instead (relays are flaky).
pub async fn members_with_adverts(dir: &Path) -> Result<Vec<MeshMember>> {
    let device = noscat_core::mesh::Device::open(dir)?;
    let members = members_of(device)?;
    let relays = relays();
    let now = nostr::prelude::Timestamp::now().as_secs();
    let infos = members.iter().map(|m| async {
        let Ok(pk) = PublicKey::parse(&m.npub) else {
            return None;
        };
        noscat_core::relay::fetch_advert_for(&relays, None, Some(pk))
            .await
            .ok()
            .flatten()
            .map(|a| (a.name, now.saturating_sub(a.created_at)))
    });
    let infos = futures_util::future::join_all(infos).await;
    Ok(members
        .into_iter()
        .zip(infos)
        .map(|(m, info)| {
            let (name, last_seen_secs) = info
                .map(|(n, s)| (n, Some(s)))
                .unwrap_or((None, None));
            MeshMember { name, last_seen_secs, ..m }
        })
        .collect())
}

/// Publish this device's MLS key package so the owner can invite it. With
/// `owner_npub`, the request is addressed to that owner alone. Returns this
/// device's npub.
pub fn prepare(dir: &Path, owner_npub: Option<&str>) -> Result<String> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let device = noscat_core::mesh::Device::open(dir)?;
    let target = owner_npub
        .map(|npub| PublicKey::parse(npub))
        .transpose()
        .map_err(|e| anyhow!("{e}"))?;
    rt.block_on(device.publish_key_package(&relays(), target))?;
    Ok(device.identity.npub())
}

/// Wait for the owner's gift-wrapped invitation and process it, giving up
/// after `timeout_secs`. `owner_npub` anchors the join: core 0.4 refuses
/// invitations from anyone else, so there is no joining without naming the
/// owner. The deadline lives inside `join_mesh` (an outer timeout would
/// drop the future mid-wait and leak the relay websocket - the leak
/// upstream fixed in 0.4.1). Returns the enriched member list on success.
pub fn join(dir: &Path, timeout_secs: u64, owner_npub: &str) -> Result<Vec<MeshMember>> {
    let expected = PublicKey::parse(owner_npub).map_err(|e| anyhow!("{e}"))?;
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let mut device = noscat_core::mesh::Device::open(dir)?;
    let relays = relays();
    rt.block_on(async move {
        device
            .join_mesh(
                &relays,
                expected,
                std::time::Duration::from_secs(timeout_secs),
            )
            .await
            .map_err(|e| {
                if matches!(e, noscat_core::mesh::MeshError::JoinTimeout) {
                    anyhow!("no invitation arrived in time")
                } else {
                    anyhow!("{e}")
                }
            })?;
        members_of(device)
    })
}

/// Merge the owner's epoch commits (invites, removals) that arrive while we
/// listen, then return the member list.
pub fn sync(dir: &Path, wait_secs: u64) -> Result<Vec<MeshMember>> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let mut device = noscat_core::mesh::Device::open(dir)?;
    let relays = relays();
    rt.block_on(async move {
        device
            .sync_mesh(&relays, std::time::Duration::from_secs(wait_secs))
            .await?;
        members_of(device)
    })
}

/// List members with advert names and presence, blocking form of
/// [`members_with_adverts`].
pub fn members(dir: &Path) -> Result<Vec<MeshMember>> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(members_with_adverts(dir))
}

/// This device's mesh identity npub. There is exactly one identity: the
/// mesh dir's `identity.nsec` is the device's master secret (Nostr, Noise,
/// and MLS all derive from it), the client runs on it, and the join screen
/// shows it - the same shape as the Android app. Opening is local-only
/// (creates the key on first run), so it is safe before any connect.
pub fn device_npub(dir: &Path) -> Result<String> {
    let device = noscat_core::mesh::Device::open(dir)?;
    Ok(device.identity.npub())
}
