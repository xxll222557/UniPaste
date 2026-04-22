use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandshakeHello {
    pub device_id: Uuid,
    pub device_name: String,
    pub timestamp_ms: u64,
    pub public_key: String,
    pub eph_public_key: String,
    pub nonce: String,
    pub signature: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageFrame {
    pub width: usize,
    pub height: usize,
    pub png_bytes: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileBundleFile {
    pub relative_path: String,
    pub byte_len: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileBundle {
    pub transfer_id: Uuid,
    pub files: Vec<FileBundleFile>,
    pub total_bytes: u64,
}

#[derive(Debug, Clone)]
pub struct LocalFileSource {
    pub source_path: PathBuf,
    pub byte_len: u64,
}

#[derive(Debug, Clone)]
pub struct ClipboardDispatch {
    pub payload: ClipboardPayload,
    pub local_files: Vec<LocalFileSource>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ClipboardContent {
    Text {
        text: String,
    },
    Html {
        html: String,
        plain_text: Option<String>,
    },
    Image {
        image: ImageFrame,
    },
    Files {
        bundle: FileBundle,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipboardPayload {
    pub message_id: Uuid,
    pub source_device_id: Uuid,
    pub created_at_ms: u64,
    pub content_hash: String,
    pub content: ClipboardContent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CipherPacket {
    pub nonce: String,
    pub ciphertext: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairRequest {
    pub request_id: Uuid,
    pub short_code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairDecision {
    pub request_id: Uuid,
    pub approved: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WireMessage {
    PairRequest(PairRequest),
    PairDecision(PairDecision),
    Clipboard(CipherPacket),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub timestamp_ms: u64,
    pub level: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub timestamp_ms: u64,
    pub direction: String,
    pub device_name: String,
    pub content_kind: String,
    pub preview: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeviceSummary {
    pub device_id: Uuid,
    pub device_name: String,
    pub fingerprint: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiscoveredDeviceSummary {
    pub device_id: Uuid,
    pub device_name: String,
    pub fingerprint: String,
    pub address: String,
    pub quic_port: u16,
    pub last_seen_ms: u64,
    pub trusted: bool,
    pub connected: bool,
    pub pending_direction: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PendingPairSummary {
    pub device_id: Uuid,
    pub device_name: String,
    pub fingerprint: String,
    pub short_code: String,
    pub direction: String,
    pub requested_at_ms: u64,
    pub expires_at_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Snapshot {
    pub local_device: DeviceSummary,
    pub discovered_devices: Vec<DiscoveredDeviceSummary>,
    pub trusted_devices: Vec<DeviceSummary>,
    pub pending_pairs: Vec<PendingPairSummary>,
    pub history_entries: Vec<HistoryEntry>,
    pub logs: Vec<LogEntry>,
    pub sync_enabled: bool,
    pub sync_html_enabled: bool,
    pub sync_images_enabled: bool,
    pub sync_files_enabled: bool,
    pub quic_port: u16,
    pub network_status: String,
    pub last_error: Option<String>,
}
