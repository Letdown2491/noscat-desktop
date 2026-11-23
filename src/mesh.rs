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

/// Wait up to `timeout_secs` for the owner's gift-wrapped invitation and
/// process it. With `owner_npub`, invitations from anyone else are ignored.
/// Returns the enriched member list on success.
pub fn join(dir: &Path, timeout_secs: u64, owner_npub: Option<&str>) -> Result<Vec<MeshMember>> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let mut device = noscat_core::mesh::Device::open(dir)?;
    let relays = relays();
    let expected = owner_npub
        .map(|npub| PublicKey::parse(npub))
        .transpose()
        .map_err(|e| anyhow!("{e}"))?;
    rt.block_on(async move {
        tokio::time::timeout(
            std::time::Duration::from_secs(timeout_secs),
            device.join_mesh(&relays, expected),
        )
        .await
        .map_err(|_| anyhow!("no invitation arrived in time"))??;
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
