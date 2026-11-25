//! The storage key that seals this device's files (identity, mesh state)
//! at rest, kept in the desktop keyring (Secret Service: GNOME Keyring,
//! KWallet, KeePassXC). Read once at start, before anything touches the
//! device directory.
//!
//! Without a keyring the files stay plain, as before, and Settings says
//! so. A key that cannot be read while the files are sealed is never
//! replaced: a new key would not open them. The device then refuses to
//! start until the keyring is back (or Forget mesh wipes it).

use std::collections::HashMap;
use std::io::Read as _;
use std::sync::OnceLock;

use secret_service::blocking::SecretService;
use secret_service::EncryptionType;

use crate::paths;

/// How the device files are stored, for Settings.
#[derive(Debug, Clone)]
pub enum Storage {
    /// Sealed with the keyring's key.
    Sealed,
    /// Plain files: no keyring to keep a key in.
    Plain(String),
    /// The files are sealed but the key cannot be read.
    Locked(String),
}

static STATUS: OnceLock<Storage> = OnceLock::new();

/// What [`init`] found.
pub fn status() -> Storage {
    STATUS.get().cloned().unwrap_or(Storage::Plain("not initialised".into()))
}

fn attributes() -> HashMap<&'static str, &'static str> {
    HashMap::from([("application", "noscat-desktop"), ("purpose", "storage-key")])
}

/// Whether the device files on disk are already sealed.
fn files_sealed() -> bool {
    ["identity.nsec", "mesh-state.json"].iter().any(|f| {
        std::fs::read(paths::mesh_dir().join(f)).is_ok_and(|d| noscat_core::fsutil::is_sealed(&d))
    })
}

fn random_key() -> std::io::Result<[u8; 32]> {
    let mut k = [0u8; 32];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut k)?;
    Ok(k)
}

/// The key from the keyring, created there on first use. `Ok(None)`: the
/// keyring has none.
fn keyring_key(create: bool) -> Result<Option<[u8; 32]>, String> {
    let ss = SecretService::connect(EncryptionType::Dh).map_err(|e| format!("no keyring ({e})"))?;
    let coll = ss.get_default_collection().map_err(|e| format!("no default keyring ({e})"))?;
    coll.ensure_unlocked().map_err(|e| format!("the keyring stayed locked ({e})"))?;
    let items = coll.search_items(attributes()).map_err(|e| e.to_string())?;
    if let Some(item) = items.first() {
        let secret = item.get_secret().map_err(|e| e.to_string())?;
        let key = <[u8; 32]>::try_from(secret.as_slice())
            .map_err(|_| "the keyring's noscat key is not 32 bytes".to_string())?;
        return Ok(Some(key));
    }
    if !create {
        return Ok(None);
    }
    let key = random_key().map_err(|e| e.to_string())?;
    coll.create_item("noscat device storage key", attributes(), &key, true, "application/octet-stream")
        .map_err(|e| format!("could not save a key in the keyring ({e})"))?;
    Ok(Some(key))
}

/// Find (or create) the key and hand it to noscat. Call once, first.
pub fn init() {
    let sealed = files_sealed();
    let status = match keyring_key(!sealed) {
        Ok(Some(key)) => {
            noscat_client::set_storage_key(Some(key));
            Storage::Sealed
        }
        Ok(None) => Storage::Locked("the keyring has no noscat key".into()),
        Err(e) if sealed => Storage::Locked(e),
        Err(e) => Storage::Plain(e),
    };
    let _ = STATUS.set(status);
}
