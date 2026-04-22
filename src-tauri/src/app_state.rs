use std::{
    collections::{HashMap, VecDeque},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU16, AtomicU64, Ordering},
        Arc,
    },
    time::{SystemTime, UNIX_EPOCH},
};

use ed25519_dalek::VerifyingKey;
use tokio::sync::{broadcast, Mutex, RwLock};
use uuid::Uuid;

use crate::{
    config::{fingerprint, LocalIdentity, TrustedPeer},
    error::AppResult,
    sync::protocol::{
        ClipboardContent, ClipboardDispatch, DeviceSummary, DiscoveredDeviceSummary, HistoryEntry,
        LogEntry, PendingPairSummary, Snapshot,
    },
};

#[derive(Debug, Clone)]
pub struct DiscoveredDevice {
    pub device_id: Uuid,
    pub device_name: String,
    pub fingerprint: String,
    pub public_key: VerifyingKey,
    pub address: String,
    pub quic_port: u16,
    pub last_seen_ms: u64,
}

#[derive(Debug, Clone)]
pub struct ConnectedPeer {
    pub device_id: Uuid,
    pub connection: quinn::Connection,
    pub session_key: [u8; 32],
    pub device_name: String,
    pub _connected_at_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairDirection {
    Outbound,
    Inbound,
}

impl PairDirection {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Outbound => "outbound",
            Self::Inbound => "inbound",
        }
    }
}

#[derive(Debug, Clone)]
pub struct PendingPair {
    pub request_id: Uuid,
    pub device_id: Uuid,
    pub device_name: String,
    pub fingerprint: String,
    pub short_code: String,
    pub direction: PairDirection,
    pub requested_at_ms: u64,
    pub expires_at_ms: u64,
}

pub struct InnerState {
    pub config_path: PathBuf,
    pub history_path: PathBuf,
    pub identity: Mutex<LocalIdentity>,
    pub trusted_peers: RwLock<HashMap<Uuid, TrustedPeer>>,
    pub discovered_peers: RwLock<HashMap<Uuid, DiscoveredDevice>>,
    pub connected_peers: RwLock<HashMap<Uuid, ConnectedPeer>>,
    pub pending_pairs: RwLock<HashMap<Uuid, PendingPair>>,
    pub history_entries: Mutex<VecDeque<HistoryEntry>>,
    pub logs: Mutex<VecDeque<LogEntry>>,
    pub last_error: Mutex<Option<String>>,
    pub sync_enabled: AtomicBool,
    pub quic_port: AtomicU16,
    pub clipboard_tx: broadcast::Sender<ClipboardDispatch>,
    pub last_local_hash: Mutex<Option<String>>,
    pub last_remote_hash: Mutex<Option<String>>,
    pub last_local_hash_at_ms: AtomicU64,
    pub last_remote_hash_at_ms: AtomicU64,
    pub suppress_until_ms: AtomicU64,
}

#[derive(Clone)]
pub struct ManagedState(pub Arc<InnerState>);

impl ManagedState {
    pub fn new(
        config_path: PathBuf,
        history_path: PathBuf,
        identity: LocalIdentity,
        trusted_peers: HashMap<Uuid, TrustedPeer>,
        history_entries: Vec<HistoryEntry>,
    ) -> Self {
        let (clipboard_tx, _) = broadcast::channel(128);
        Self(Arc::new(InnerState {
            config_path,
            history_path,
            sync_enabled: AtomicBool::new(identity.sync_enabled),
            identity: Mutex::new(identity),
            trusted_peers: RwLock::new(trusted_peers),
            discovered_peers: RwLock::new(HashMap::new()),
            connected_peers: RwLock::new(HashMap::new()),
            pending_pairs: RwLock::new(HashMap::new()),
            history_entries: Mutex::new(history_entries.into_iter().collect()),
            logs: Mutex::new(VecDeque::with_capacity(200)),
            last_error: Mutex::new(None),
            quic_port: AtomicU16::new(0),
            clipboard_tx,
            last_local_hash: Mutex::new(None),
            last_remote_hash: Mutex::new(None),
            last_local_hash_at_ms: AtomicU64::new(0),
            last_remote_hash_at_ms: AtomicU64::new(0),
            suppress_until_ms: AtomicU64::new(0),
        }))
    }

    pub async fn snapshot(&self) -> Snapshot {
        let identity = self.0.identity.lock().await.clone();
        let trusted = self.0.trusted_peers.read().await;
        let discovered = self.0.discovered_peers.read().await;
        let connected = self.0.connected_peers.read().await;
        let pending = self.0.pending_pairs.read().await;
        let history_entries = self.0.history_entries.lock().await;
        let logs = self.0.logs.lock().await;
        let last_error = self.0.last_error.lock().await.clone();

        let trusted_devices = trusted
            .values()
            .map(|peer| DeviceSummary {
                device_id: peer.device_id,
                device_name: peer.device_name.clone(),
                fingerprint: fingerprint(&peer.public_key.to_bytes()),
            })
            .collect::<Vec<_>>();

        let mut pending_pairs = pending
            .values()
            .map(|pair| PendingPairSummary {
                device_id: pair.device_id,
                device_name: pair.device_name.clone(),
                fingerprint: pair.fingerprint.clone(),
                short_code: pair.short_code.clone(),
                direction: pair.direction.as_str().to_string(),
                requested_at_ms: pair.requested_at_ms,
                expires_at_ms: pair.expires_at_ms,
            })
            .collect::<Vec<_>>();
        pending_pairs.sort_by(|a, b| b.requested_at_ms.cmp(&a.requested_at_ms));

        let mut discovered_devices = discovered
            .values()
            .map(|peer| DiscoveredDeviceSummary {
                device_id: peer.device_id,
                device_name: peer.device_name.clone(),
                fingerprint: peer.fingerprint.clone(),
                address: peer.address.clone(),
                quic_port: peer.quic_port,
                last_seen_ms: peer.last_seen_ms,
                trusted: trusted.contains_key(&peer.device_id),
                connected: connected.contains_key(&peer.device_id),
                pending_direction: pending
                    .get(&peer.device_id)
                    .map(|pair| pair.direction.as_str().to_string()),
            })
            .collect::<Vec<_>>();
        discovered_devices.sort_by(|a, b| b.last_seen_ms.cmp(&a.last_seen_ms));

        Snapshot {
            local_device: DeviceSummary {
                device_id: identity.device_id,
                device_name: identity.device_name,
                fingerprint: identity.fingerprint,
            },
            discovered_devices,
            trusted_devices,
            pending_pairs,
            history_entries: history_entries.iter().cloned().collect(),
            logs: logs.iter().cloned().collect(),
            sync_enabled: self.0.sync_enabled.load(Ordering::Relaxed),
            sync_html_enabled: identity.sync_html,
            sync_images_enabled: identity.sync_images,
            sync_files_enabled: identity.sync_files,
            quic_port: self.0.quic_port.load(Ordering::Relaxed),
            network_status: if !self.0.sync_enabled.load(Ordering::Relaxed) {
                "paused".to_string()
            } else if !connected.is_empty() {
                "connected".to_string()
            } else if !discovered.is_empty() {
                "discovered".to_string()
            } else if self.0.quic_port.load(Ordering::Relaxed) > 0 {
                "ready".to_string()
            } else {
                "starting".to_string()
            },
            last_error,
        }
    }

    pub async fn log(&self, level: &str, message: impl Into<String>) {
        let mut logs = self.0.logs.lock().await;
        if logs.len() >= 160 {
            logs.pop_front();
        }
        logs.push_back(LogEntry {
            timestamp_ms: now_ms(),
            level: level.to_string(),
            message: message.into(),
        });
    }

    pub async fn set_last_error(&self, message: Option<String>) {
        *self.0.last_error.lock().await = message;
    }

    pub async fn report_error(&self, message: impl Into<String>) {
        let message = message.into();
        self.set_last_error(Some(message.clone())).await;
        self.log("ERROR", message).await;
    }

    pub async fn push_history(
        &self,
        direction: &str,
        device_name: impl Into<String>,
        content_kind: &str,
        preview: impl Into<String>,
    ) {
        let mut history = self.0.history_entries.lock().await;
        if history.len() >= 80 {
            history.pop_front();
        }
        history.push_back(HistoryEntry {
            timestamp_ms: now_ms(),
            direction: direction.to_string(),
            device_name: device_name.into(),
            content_kind: content_kind.to_string(),
            preview: preview.into(),
        });
        let entries = history.iter().cloned().collect::<Vec<_>>();
        drop(history);
        let _ = crate::config::persist_history(&self.0.history_path, &entries);
    }

    pub async fn clear_history(&self) -> AppResult<()> {
        self.0.history_entries.lock().await.clear();
        crate::config::persist_history(&self.0.history_path, &[])
    }

    pub async fn persist(&self) -> AppResult<()> {
        let mut identity = self.0.identity.lock().await;
        identity.sync_enabled = self.0.sync_enabled.load(Ordering::Relaxed);
        let trusted = self.0.trusted_peers.read().await;
        crate::config::persist(&self.0.config_path, &identity, &trusted)
    }

    pub async fn set_trusted_peer(&self, peer: TrustedPeer) -> AppResult<()> {
        self.0.pending_pairs.write().await.remove(&peer.device_id);
        self.0.trusted_peers.write().await.insert(peer.device_id, peer);
        self.persist().await
    }

    pub async fn remove_trusted_peer(&self, device_id: Uuid) -> AppResult<()> {
        if let Some(peer) = self.0.connected_peers.write().await.remove(&device_id) {
            peer.connection.close(0u32.into(), b"peer removed");
        }
        self.0.pending_pairs.write().await.remove(&device_id);
        self.0.trusted_peers.write().await.remove(&device_id);
        self.persist().await
    }

    pub async fn update_device_name(&self, device_name: String) -> AppResult<()> {
        let mut identity = self.0.identity.lock().await;
        identity.device_name = device_name;
        drop(identity);
        self.persist().await
    }

    pub async fn update_sync_preferences(
        &self,
        sync_html: bool,
        sync_images: bool,
        sync_files: bool,
    ) -> AppResult<()> {
        let mut identity = self.0.identity.lock().await;
        identity.sync_html = sync_html;
        identity.sync_images = sync_images;
        identity.sync_files = sync_files;
        drop(identity);
        self.persist().await
    }

    pub async fn sync_allows(&self, content: &ClipboardContent) -> bool {
        let identity = self.0.identity.lock().await;
        match content {
            ClipboardContent::Text { .. } => true,
            ClipboardContent::Html { .. } => identity.sync_html,
            ClipboardContent::Image { .. } => identity.sync_images,
            ClipboardContent::Files { .. } => identity.sync_files,
        }
    }
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
