//! This app's own noscat identity: a generated keypair stored under the
//! app's data dir (mode 0600), deliberately separate from the CLI's
//! `~/.noscat` identity so the desktop app is its own device in the mesh.
//! The same file the client runtime consumes via `IdentitySource::File`;
//! generating here too lets the join view show the npub before connecting.

use std::io::Write as _;
use std::os::unix::fs::OpenOptionsExt as _;

use anyhow::{Context as _, Result, anyhow};
use noscat_core::Identity;

/// Ensure the identity file exists (generating it on first run) and return
/// the npub it addresses.
pub fn ensure_identity() -> Result<String> {
    let path = crate::paths::identity_file();
    std::fs::create_dir_all(path.parent().expect("identity path has a parent"))?;
    let identity = if path.exists() {
        let secret = std::fs::read_to_string(&path)?;
        Identity::from_secret(secret.trim()).map_err(|e| anyhow!("{e}"))?
    } else {
        let identity = Identity::generate();
        let mut file = std::fs::OpenOptions::new()
            .mode(0o600)
            .write(true)
            .create_new(true)
            .open(&path)
            .with_context(|| format!("cannot create {}", path.display()))?;
        file.write_all(identity.secret_hex().as_bytes())?;
        identity
    };
    Ok(identity.npub())
}
