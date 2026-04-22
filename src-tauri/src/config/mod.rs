use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use dirs::config_dir;
use ed25519_dalek::{SigningKey, VerifyingKey};
use keyring::Entry;
use rand_core::OsRng;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    error::{AppError, AppResult},
    sync::protocol::HistoryEntry,
};

#[derive(Debug, Clone)]
pub struct TrustedPeer {
    pub device_id: Uuid,
    pub device_name: String,
    pub public_key: VerifyingKey,
}

#[derive(Debug, Clone)]
pub struct LocalIdentity {
    pub device_id: Uuid,
    pub device_name: String,
    pub signing_key: SigningKey,
    pub fingerprint: String,
    pub sync_enabled: bool,
    pub sync_html: bool,
    pub sync_images: bool,
    pub sync_files: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredConfig {
    device_id: Uuid,
    device_name: String,
    #[serde(default)]
    secret_key: Option<String>,
    #[serde(default = "default_true")]
    sync_enabled: bool,
    #[serde(default = "default_true")]
    sync_html: bool,
    #[serde(default = "default_true")]
    sync_images: bool,
    #[serde(default = "default_true")]
    sync_files: bool,
    #[serde(default)]
    trusted_peers: Vec<StoredTrustedPeer>,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredTrustedPeer {
    device_id: Uuid,
    device_name: String,
    public_key: String,
}

pub struct ConfigBundle {
    pub path: PathBuf,
    pub history_path: PathBuf,
    pub identity: LocalIdentity,
    pub trusted_peers: HashMap<Uuid, TrustedPeer>,
    pub history_entries: Vec<HistoryEntry>,
}

struct LoadedSigningKey {
    signing_key: SigningKey,
    stored_secret_key: Option<String>,
}

pub fn load_or_create() -> AppResult<ConfigBundle> {
    let root = config_root()?;
    fs::create_dir_all(&root)?;
    let path = root.join("config.json");
    let history_path = root.join("history.json");

    if !path.exists() {
        let device_id = Uuid::new_v4();
        let signing_key = SigningKey::generate(&mut OsRng);
        let device_name = default_device_name();
        let mut stored = StoredConfig {
            device_id,
            device_name,
            secret_key: None,
            sync_enabled: true,
            sync_html: true,
            sync_images: true,
            sync_files: true,
            trusted_peers: Vec::new(),
        };
        if store_signing_key(device_id, &signing_key).is_err() {
            stored.secret_key = Some(STANDARD.encode(signing_key.to_bytes()));
        }
        write_config(&path, &stored)?;
    }

    let raw = fs::read_to_string(&path)?;
    let mut stored: StoredConfig = serde_json::from_str(&raw)?;
    let loaded_key = load_signing_key(stored.device_id, stored.secret_key.as_deref())?;
    if stored.secret_key != loaded_key.stored_secret_key {
        stored.secret_key = loaded_key.stored_secret_key.clone();
        write_config(&path, &stored)?;
    }
    let signing_key = loaded_key.signing_key;
    let verifying_key = signing_key.verifying_key();
    let trusted_peers = stored
        .trusted_peers
        .into_iter()
        .map(|peer| {
            let public_key = VerifyingKey::from_bytes(&decode_32(&peer.public_key)?)
                .map_err(|error| AppError::Crypto(error.to_string()))?;
            Ok((
                peer.device_id,
                TrustedPeer {
                    device_id: peer.device_id,
                    device_name: peer.device_name,
                    public_key,
                },
            ))
        })
        .collect::<AppResult<HashMap<_, _>>>()?;
    let history_entries = load_history(&history_path)?;

    Ok(ConfigBundle {
        path,
        history_path,
        identity: LocalIdentity {
            device_id: stored.device_id,
            device_name: stored.device_name,
            signing_key,
            fingerprint: fingerprint(&verifying_key.to_bytes()),
            sync_enabled: stored.sync_enabled,
            sync_html: stored.sync_html,
            sync_images: stored.sync_images,
            sync_files: stored.sync_files,
        },
        trusted_peers,
        history_entries,
    })
}

pub fn persist(
    path: &Path,
    identity: &LocalIdentity,
    trusted_peers: &HashMap<Uuid, TrustedPeer>,
) -> AppResult<()> {
    let stored = StoredConfig {
        device_id: identity.device_id,
        device_name: identity.device_name.clone(),
        secret_key: persist_signing_key(identity),
        sync_enabled: identity.sync_enabled,
        sync_html: identity.sync_html,
        sync_images: identity.sync_images,
        sync_files: identity.sync_files,
        trusted_peers: trusted_peers
            .values()
            .map(|peer| StoredTrustedPeer {
                device_id: peer.device_id,
                device_name: peer.device_name.clone(),
                public_key: STANDARD.encode(peer.public_key.to_bytes()),
            })
            .collect(),
    };
    write_config(path, &stored)
}

fn write_config(path: &Path, stored: &StoredConfig) -> AppResult<()> {
    let payload = serde_json::to_string_pretty(stored)?;
    fs::write(path, payload)?;
    Ok(())
}

pub fn persist_history(path: &Path, entries: &[HistoryEntry]) -> AppResult<()> {
    let payload = serde_json::to_string_pretty(entries)?;
    fs::write(path, payload)?;
    Ok(())
}

fn load_history(path: &Path) -> AppResult<Vec<HistoryEntry>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = fs::read_to_string(path)?;
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_str(&raw).map_err(AppError::from)
}

fn config_root() -> AppResult<PathBuf> {
    let base = config_dir().ok_or_else(|| AppError::Invalid("config dir unavailable".into()))?;
    Ok(base.join("UniPaste"))
}

fn load_signing_key(device_id: Uuid, legacy_secret_key: Option<&str>) -> AppResult<LoadedSigningKey> {
    if let Ok(secret_key) = read_signing_key(device_id) {
        return Ok(LoadedSigningKey {
            signing_key: secret_key,
            stored_secret_key: None,
        });
    }

    if let Some(legacy_secret_key) = legacy_secret_key {
        let secret_key = decode_32(legacy_secret_key)?;
        let signing_key = SigningKey::from_bytes(&secret_key);
        let stored_secret_key = if store_signing_key(device_id, &signing_key).is_ok() {
            None
        } else {
            Some(legacy_secret_key.to_string())
        };
        return Ok(LoadedSigningKey {
            signing_key,
            stored_secret_key,
        });
    }

    let signing_key = SigningKey::generate(&mut OsRng);
    let stored_secret_key = if store_signing_key(device_id, &signing_key).is_ok() {
        None
    } else {
        Some(STANDARD.encode(signing_key.to_bytes()))
    };
    Ok(LoadedSigningKey {
        signing_key,
        stored_secret_key,
    })
}

fn persist_signing_key(identity: &LocalIdentity) -> Option<String> {
    if store_signing_key(identity.device_id, &identity.signing_key).is_ok() {
        None
    } else {
        Some(STANDARD.encode(identity.signing_key.to_bytes()))
    }
}

fn store_signing_key(device_id: Uuid, signing_key: &SigningKey) -> AppResult<()> {
    let entry = Entry::new(KEYRING_SERVICE, &keyring_username(device_id))
        .map_err(|error| AppError::Crypto(error.to_string()))?;
    entry
        .set_password(&STANDARD.encode(signing_key.to_bytes()))
        .map_err(|error| AppError::Crypto(error.to_string()))
}

fn read_signing_key(device_id: Uuid) -> AppResult<SigningKey> {
    let entry = Entry::new(KEYRING_SERVICE, &keyring_username(device_id))
        .map_err(|error| AppError::Crypto(error.to_string()))?;
    let encoded = entry
        .get_password()
        .map_err(|error| AppError::Crypto(error.to_string()))?;
    let secret_key = decode_32(&encoded)?;
    Ok(SigningKey::from_bytes(&secret_key))
}

fn decode_32(value: &str) -> AppResult<[u8; 32]> {
    let bytes = STANDARD.decode(value)?;
    let array: [u8; 32] = bytes
        .try_into()
        .map_err(|_| AppError::Invalid("expected 32-byte key".into()))?;
    Ok(array)
}

fn default_device_name() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| "UniPaste Device".to_string())
}

pub fn fingerprint(public_key: &[u8]) -> String {
    let hash = blake3::hash(public_key).to_hex().to_string();
    hash[0..12].to_uppercase()
}

fn keyring_username(device_id: Uuid) -> String {
    format!("identity:{device_id}")
}

fn default_true() -> bool {
    true
}

const KEYRING_SERVICE: &str = "UniPaste";
