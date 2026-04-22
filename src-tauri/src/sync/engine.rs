use std::{collections::HashSet, net::SocketAddr, sync::atomic::Ordering, time::Duration};

use quinn::{ConnectionError, Endpoint, RecvStream, SendStream};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tokio::io::AsyncReadExt;
use tokio::time::{interval, sleep, timeout, MissedTickBehavior};
use uuid::Uuid;

use crate::{
    app_state::{now_ms, ConnectedPeer, ManagedState, PairDirection, PendingPair},
    clipboard,
    config::TrustedPeer,
    crypto,
    error::{AppError, AppResult},
    sync::protocol::{ClipboardPayload, FileBundle, LocalFileSource, PairDecision, PairRequest, WireMessage},
};

const MAX_FRAME_SIZE: usize = 16 * 1024 * 1024;
const PAIR_REQUEST_TTL_MS: u64 = 90_000;
const DISCOVERY_TTL_MS: u64 = 180_000;
const CLIPBOARD_DEDUP_WINDOW_MS: u64 = 2_500;
const FAST_POLL_MS: u64 = 180;
const IDLE_POLL_MS: u64 = 700;
const PAUSED_POLL_MS: u64 = 1_000;
const FILE_CHUNK_SIZE: usize = 64 * 1024;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum StreamEnvelope {
    Control { message: WireMessage },
    FileTransfer { manifest: crate::sync::protocol::CipherPacket },
}

pub fn spawn_runtime(state: ManagedState) {
    let runtime_state = state.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = run_quic(runtime_state.clone()).await {
            runtime_state
                .report_error(format!("QUIC 运行时启动失败: {error}"))
                .await;
        }
    });

    let monitor_state = state.clone();
    tauri::async_runtime::spawn(async move {
        run_clipboard_monitor(monitor_state).await;
    });

    let dispatch_state = state.clone();
    tauri::async_runtime::spawn(async move {
        run_clipboard_dispatcher(dispatch_state).await;
    });

    let housekeeping_state = state.clone();
    tauri::async_runtime::spawn(async move {
        run_housekeeping_loop(housekeeping_state).await;
    });
}

async fn run_quic(state: ManagedState) -> AppResult<()> {
    let server_config = crypto::build_quic_server_config()?;
    let client_config = crypto::build_quic_client_config()?;
    let bind_addr = SocketAddr::from(([0, 0, 0, 0], 0));
    let mut endpoint = Endpoint::server(server_config, bind_addr).map_err(|error| AppError::Io(error))?;
    endpoint.set_default_client_config(client_config);
    let local_addr = endpoint.local_addr().map_err(|error| AppError::Io(error))?;
    state.0.quic_port.store(local_addr.port(), Ordering::Relaxed);
    state
        .log("INFO", format!("QUIC 同步端点已启动: {}", local_addr.port()))
        .await;
    state.set_last_error(None).await;

    let accept_state = state.clone();
    let accept_endpoint = endpoint.clone();
    tauri::async_runtime::spawn(async move {
        run_accept_loop(accept_state, accept_endpoint).await;
    });

    run_connector_loop(state, endpoint).await;
    Ok(())
}

async fn run_accept_loop(state: ManagedState, endpoint: Endpoint) {
    while let Some(incoming) = endpoint.accept().await {
        let state_clone = state.clone();
        tauri::async_runtime::spawn(async move {
            match incoming.await {
                Ok(connection) => {
                    if let Err(error) = handle_connection(state_clone.clone(), connection, false).await {
                        state_clone.log("WARN", format!("入站 QUIC 会话结束: {error}")).await;
                    }
                }
                Err(error) => {
                    state_clone
                        .report_error(format!("接收入站 QUIC 连接失败: {error}"))
                        .await;
                }
            }
        });
    }
}

async fn run_connector_loop(state: ManagedState, endpoint: Endpoint) {
    let mut ticker = interval(Duration::from_secs(3));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        ticker.tick().await;

        let local_id = state.0.identity.lock().await.device_id;
        let peers = {
            let discovered = state.0.discovered_peers.read().await;
            let trusted = state.0.trusted_peers.read().await;
            let connected = state.0.connected_peers.read().await;
            let pending = state.0.pending_pairs.read().await;
            discovered
                .values()
                .filter(|peer| {
                    trusted.contains_key(&peer.device_id)
                        || pending
                            .get(&peer.device_id)
                            .is_some_and(|pair| pair.direction == PairDirection::Outbound)
                })
                .filter(|peer| !connected.contains_key(&peer.device_id))
                .filter(|peer| {
                    pending
                        .get(&peer.device_id)
                        .is_some_and(|pair| pair.direction == PairDirection::Outbound)
                        || local_id.as_bytes() > peer.device_id.as_bytes()
                })
                .map(|peer| (peer.device_id, peer.address.clone(), peer.quic_port))
                .collect::<Vec<_>>()
        };

        for (device_id, address, port) in peers {
            let state_clone = state.clone();
            let endpoint_clone = endpoint.clone();
            tauri::async_runtime::spawn(async move {
                let Ok(socket_addr) = format!("{address}:{port}").parse::<SocketAddr>() else {
                    state_clone
                        .log("WARN", format!("无效的设备地址: {address}:{port}"))
                        .await;
                    return;
                };

                let connecting = match endpoint_clone.connect(socket_addr, "unipaste.local") {
                    Ok(connecting) => connecting,
                    Err(error) => {
                        state_clone
                            .log("WARN", format!("连接 {device_id} 失败: {error}"))
                            .await;
                        return;
                    }
                };

                match timeout(Duration::from_secs(4), connecting).await {
                    Ok(Ok(connection)) => {
                        if let Err(error) = handle_connection(state_clone.clone(), connection, true).await {
                            state_clone
                                .log("WARN", format!("连接 {device_id} 失败: {error}"))
                                .await;
                        }
                    }
                    Ok(Err(error)) => {
                        state_clone
                            .log("WARN", format!("连接 {device_id} 失败: {error}"))
                            .await;
                    }
                    Err(_) => {
                        state_clone
                            .log("WARN", format!("连接 {device_id} 超时"))
                            .await;
                    }
                }
            });
        }
    }
}

async fn handle_connection(state: ManagedState, connection: quinn::Connection, initiated_by_us: bool) -> AppResult<()> {
    let (remote_hello, session_key) = handshake(&state, &connection, initiated_by_us).await?;
    let remote_key = crypto::verify_handshake(&remote_hello)?;
    let remote_device_id = remote_hello.device_id;
    let remote_fingerprint = crate::config::fingerprint(&remote_key.to_bytes());

    {
        let trusted = state.0.trusted_peers.read().await;
        if let Some(expected) = trusted.get(&remote_device_id) {
            if expected.public_key != remote_key {
                return Err(AppError::Crypto("trusted device key mismatch".into()));
            }
        }
    }

    state.0.connected_peers.write().await.insert(
        remote_device_id,
        ConnectedPeer {
            device_id: remote_device_id,
            connection: connection.clone(),
            session_key,
            device_name: remote_hello.device_name.clone(),
            _connected_at_ms: now_ms(),
        },
    );
    state
        .log(
            "INFO",
            format!("已建立 QUIC 会话: {} ({remote_fingerprint})", remote_hello.device_name),
        )
        .await;
    state.set_last_error(None).await;

    maybe_send_pair_request(&state, remote_device_id).await?;

    loop {
        match connection.accept_bi().await {
            Ok((send, recv)) => {
                let state_clone = state.clone();
                let connection_clone = connection.clone();
                let remote_name = remote_hello.device_name.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(error) = handle_stream(
                        state_clone.clone(),
                        connection_clone,
                        remote_device_id,
                        remote_name,
                        remote_key,
                        session_key,
                        send,
                        recv,
                    )
                    .await
                    {
                        state_clone
                            .log("WARN", format!("处理设备消息失败: {error}"))
                            .await;
                    }
                });
            }
            Err(ConnectionError::ApplicationClosed { .. }) => break,
            Err(ConnectionError::LocallyClosed) => break,
            Err(ConnectionError::ConnectionClosed(_)) => break,
            Err(error) => return Err(AppError::Invalid(error.to_string())),
        }
    }

    state.0.connected_peers.write().await.remove(&remote_device_id);
    state
        .log("INFO", format!("会话结束: {}", remote_hello.device_name))
        .await;
    Ok(())
}

async fn handshake(
    state: &ManagedState,
    connection: &quinn::Connection,
    initiated_by_us: bool,
) -> AppResult<(crate::sync::protocol::HandshakeHello, [u8; 32])> {
    let identity = state.0.identity.lock().await.clone();
    let handshake = crypto::build_handshake(identity.device_id, &identity.device_name, &identity.signing_key);

    let remote_hello = if initiated_by_us {
        let (mut send, mut recv) = connection
            .open_bi()
            .await
            .map_err(|error| AppError::Invalid(error.to_string()))?;
        write_json(&mut send, &handshake.hello).await?;
        let remote = read_json::<crate::sync::protocol::HandshakeHello>(&mut recv).await?;
        remote
    } else {
        let (mut send, mut recv) = connection
            .accept_bi()
            .await
            .map_err(|error| AppError::Invalid(error.to_string()))?;
        let remote = read_json::<crate::sync::protocol::HandshakeHello>(&mut recv).await?;
        write_json(&mut send, &handshake.hello).await?;
        remote
    };

    let session_key = crypto::derive_session_key(handshake.local_secret, &remote_hello.eph_public_key)?;
    Ok((remote_hello, session_key))
}

async fn handle_stream(
    state: ManagedState,
    connection: quinn::Connection,
    remote_device_id: Uuid,
    remote_name: String,
    remote_key: ed25519_dalek::VerifyingKey,
    session_key: [u8; 32],
    _send: SendStream,
    mut recv: RecvStream,
) -> AppResult<()> {
    let envelope = read_stream_envelope(&mut recv).await?;
    match envelope {
        StreamEnvelope::Control { message } => match message {
            WireMessage::PairRequest(request) => {
                let local_public = state.0.identity.lock().await.signing_key.verifying_key().to_bytes();
                let derived_code = crypto::pairing_code(&local_public, &remote_key.to_bytes());
                if derived_code != request.short_code {
                    state
                        .log(
                            "WARN",
                            format!("设备 {remote_name} 的配对码不匹配，仍展示本地计算值"),
                        )
                        .await;
                }
                state.0.pending_pairs.write().await.insert(
                    remote_device_id,
                    PendingPair {
                        request_id: request.request_id,
                        device_id: remote_device_id,
                        device_name: remote_name.clone(),
                        fingerprint: crate::config::fingerprint(&remote_key.to_bytes()),
                        short_code: derived_code,
                        direction: PairDirection::Inbound,
                        requested_at_ms: now_ms(),
                        expires_at_ms: now_ms() + PAIR_REQUEST_TTL_MS,
                    },
                );
                state
                    .log("INFO", format!("收到来自 {remote_name} 的配对请求"))
                    .await;
            }
            WireMessage::PairDecision(decision) => {
                handle_pair_decision(state, remote_device_id, remote_name, remote_key, decision).await?;
            }
            WireMessage::Clipboard(packet) => {
                let plaintext = crypto::decrypt(&session_key, &packet.nonce, &packet.ciphertext)?;
                let payload: ClipboardPayload = serde_json::from_slice(&plaintext)?;
                handle_remote_clipboard(state, remote_device_id, payload).await?;
            }
        },
        StreamEnvelope::FileTransfer { manifest } => {
            let plaintext = crypto::decrypt(&session_key, &manifest.nonce, &manifest.ciphertext)?;
            let bundle: FileBundle = serde_json::from_slice(&plaintext)?;
            receive_file_transfer(state, remote_name, bundle, &mut recv).await?;
        }
    }

    let _ = connection;
    Ok(())
}

async fn handle_pair_decision(
    state: ManagedState,
    remote_device_id: Uuid,
    remote_name: String,
    remote_key: ed25519_dalek::VerifyingKey,
    decision: PairDecision,
) -> AppResult<()> {
    let pending = state.0.pending_pairs.read().await.get(&remote_device_id).cloned();
    if let Some(pair) = pending {
        if pair.request_id != decision.request_id {
            return Ok(());
        }
    }

    if decision.approved {
        state
            .set_trusted_peer(TrustedPeer {
                device_id: remote_device_id,
                device_name: remote_name.clone(),
                public_key: remote_key,
            })
            .await?;
        state.0.pending_pairs.write().await.remove(&remote_device_id);
        state
            .log("INFO", format!("设备 {remote_name} 已确认配对，现已互信"))
            .await;
    } else {
        state.0.pending_pairs.write().await.remove(&remote_device_id);
        state
            .log("INFO", format!("设备 {remote_name} 拒绝了配对请求"))
            .await;
    }
    Ok(())
}

async fn maybe_send_pair_request(state: &ManagedState, remote_device_id: Uuid) -> AppResult<()> {
    let pending = state
        .0
        .pending_pairs
        .read()
        .await
        .get(&remote_device_id)
        .cloned();
    let Some(pair) = pending else {
        return Ok(());
    };

    if pair.direction != PairDirection::Outbound {
        return Ok(());
    }

    send_wire_to_peer(
        state,
        remote_device_id,
        WireMessage::PairRequest(PairRequest {
            request_id: pair.request_id,
            short_code: pair.short_code,
        }),
    )
    .await
}

async fn run_clipboard_dispatcher(state: ManagedState) {
    let mut rx = state.0.clipboard_tx.subscribe();
    loop {
        let Ok(dispatch) = rx.recv().await else {
            continue;
        };

        if !state.0.sync_enabled.load(Ordering::Relaxed) || !state.sync_allows(&dispatch.payload.content).await {
            continue;
        }

        let preview = clipboard::content_preview(&dispatch.payload.content);
        let kind = clipboard::content_kind_label(&dispatch.payload.content);

        let peers = state.0.connected_peers.read().await.values().cloned().collect::<Vec<_>>();
        for peer in peers {
            let trusted = state.0.trusted_peers.read().await.contains_key(&peer.device_id);
            if !trusted {
                continue;
            }

            if !dispatch.local_files.is_empty()
                && send_file_transfer(
                    &peer.connection,
                    &peer.session_key,
                    &dispatch.payload,
                    &dispatch.local_files,
                )
                .await
                .is_err()
            {
                state
                    .log("WARN", format!("发送给 {} 的文件流失败", peer.device_name))
                    .await;
                continue;
            }

            let message = match serde_json::to_vec(&dispatch.payload) {
                Ok(bytes) => bytes,
                Err(_) => continue,
            };
            let Ok((nonce, ciphertext)) = crypto::encrypt(&peer.session_key, &message) else {
                continue;
            };
            let wire = WireMessage::Clipboard(crate::sync::protocol::CipherPacket { nonce, ciphertext });
            if send_wire_message(&peer.connection, &wire).await.is_ok() {
                state
                    .push_history("sent", peer.device_name.clone(), kind, preview.clone())
                    .await;
            } else {
                state
                    .log("WARN", format!("发送给 {} 的同步消息失败", peer.device_name))
                    .await;
            }
        }
    }
}

async fn run_clipboard_monitor(state: ManagedState) {
    state
        .log("INFO", "剪贴板监视器已启动，支持文本 / HTML / 图片 / 文件同步")
        .await;
    let mut fast_mode_until = 0u64;
    loop {
        let delay_ms = if !state.0.sync_enabled.load(Ordering::Relaxed) {
            PAUSED_POLL_MS
        } else if now_ms() < fast_mode_until {
            FAST_POLL_MS
        } else {
            IDLE_POLL_MS
        };
        clipboard::wait_for_change(Duration::from_millis(delay_ms)).await;

        if !state.0.sync_enabled.load(Ordering::Relaxed) {
            continue;
        }

        let local_device_id = state.0.identity.lock().await.device_id;
        let Some(dispatch) = (match clipboard::read_content(local_device_id).await {
            Ok(value) => value,
            Err(error) => {
                state.log("WARN", format!("读取剪贴板失败: {error}")).await;
                continue;
            }
        }) else {
            continue;
        };

        if !state.sync_allows(&dispatch.payload.content).await {
            continue;
        }

        let hash = dispatch.payload.content_hash.clone();
        let now = now_ms();

        let suppress_until = state.0.suppress_until_ms.load(Ordering::Relaxed);
        let last_remote_hash = state.0.last_remote_hash.lock().await.clone();
        if now < suppress_until && last_remote_hash.as_deref() == Some(hash.as_str()) {
            continue;
        }

        let mut last_local_hash = state.0.last_local_hash.lock().await;
        let last_local_hash_at_ms = state.0.last_local_hash_at_ms.load(Ordering::Relaxed);
        if last_local_hash.as_deref() == Some(hash.as_str())
            && now.saturating_sub(last_local_hash_at_ms) < CLIPBOARD_DEDUP_WINDOW_MS
        {
            continue;
        }
        *last_local_hash = Some(hash.clone());
        state.0.last_local_hash_at_ms.store(now, Ordering::Relaxed);
        drop(last_local_hash);

        if state.0.clipboard_tx.send(dispatch.clone()).is_ok() {
            fast_mode_until = now + 3_000;
            state
                .log(
                    "INFO",
                    format!(
                        "检测到本地剪贴板更新，已广播 {}",
                        clipboard::content_preview(&dispatch.payload.content)
                    ),
                )
                .await;
        }
    }
}

async fn handle_remote_clipboard(
    state: ManagedState,
    remote_device_id: Uuid,
    payload: ClipboardPayload,
) -> AppResult<()> {
    if !state.0.sync_enabled.load(Ordering::Relaxed) {
        return Ok(());
    }

    let local_device_id = state.0.identity.lock().await.device_id;
    if payload.source_device_id == local_device_id {
        return Ok(());
    }

    if !is_trusted(&state, remote_device_id).await {
        state.log("WARN", format!("忽略未信任设备 {remote_device_id} 的剪贴板消息")).await;
        return Ok(());
    }

    {
        let last_local_hash = state.0.last_local_hash.lock().await.clone();
        let last_local_hash_at_ms = state.0.last_local_hash_at_ms.load(Ordering::Relaxed);
        if last_local_hash.as_deref() == Some(payload.content_hash.as_str())
            && now_ms().saturating_sub(last_local_hash_at_ms) < CLIPBOARD_DEDUP_WINDOW_MS
        {
            return Ok(());
        }
    }

    let preview = clipboard::content_preview(&payload.content);
    let kind = clipboard::content_kind_label(&payload.content);
    if !state.sync_allows(&payload.content).await {
        return Ok(());
    }
    if let crate::sync::protocol::ClipboardContent::Files { bundle } = &payload.content {
        wait_for_received_files(bundle).await?;
    }
    clipboard::write_content(payload.content, Some(payload.message_id.to_string())).await?;
    {
        let mut last_remote_hash = state.0.last_remote_hash.lock().await;
        *last_remote_hash = Some(payload.content_hash.clone());
    }
    state
        .0
        .last_remote_hash_at_ms
        .store(now_ms(), Ordering::Relaxed);
    {
        let mut last_local_hash = state.0.last_local_hash.lock().await;
        *last_local_hash = Some(payload.content_hash.clone());
    }
    state
        .0
        .last_local_hash_at_ms
        .store(now_ms(), Ordering::Relaxed);
    state
        .0
        .suppress_until_ms
        .store(now_ms() + 1_400, Ordering::Relaxed);
    let remote_name = state
        .0
        .discovered_peers
        .read()
        .await
        .get(&remote_device_id)
        .map(|peer| peer.device_name.clone())
        .unwrap_or_else(|| remote_device_id.to_string());
    state
        .push_history("received", remote_name, kind, preview.clone())
        .await;
    state
        .log("INFO", format!("已应用来自 {remote_device_id} 的剪贴板内容: {preview}"))
        .await;
    Ok(())
}

async fn is_trusted(state: &ManagedState, device_id: Uuid) -> bool {
    state.0.trusted_peers.read().await.contains_key(&device_id)
}

pub async fn create_outbound_pair(state: &ManagedState, device_id: Uuid) -> AppResult<PendingPair> {
    let discovered = state.0.discovered_peers.read().await;
    let peer = discovered
        .get(&device_id)
        .cloned()
        .ok_or_else(|| AppError::Invalid("device not found".into()))?;
    let local_public = state.0.identity.lock().await.signing_key.verifying_key().to_bytes();
    let short_code = crypto::pairing_code(&local_public, &peer.public_key.to_bytes());
    let pair = PendingPair {
        request_id: Uuid::new_v4(),
        device_id,
        device_name: peer.device_name,
        fingerprint: peer.fingerprint,
        short_code,
        direction: PairDirection::Outbound,
        requested_at_ms: now_ms(),
        expires_at_ms: now_ms() + PAIR_REQUEST_TTL_MS,
    };
    state.0.pending_pairs.write().await.insert(device_id, pair.clone());
    Ok(pair)
}

pub async fn approve_pair(state: &ManagedState, device_id: Uuid, short_code: &str) -> AppResult<()> {
    let pending = state
        .0
        .pending_pairs
        .read()
        .await
        .get(&device_id)
        .cloned()
        .ok_or_else(|| AppError::Invalid("pair request not found".into()))?;
    if pending.expires_at_ms < now_ms() {
        state.0.pending_pairs.write().await.remove(&device_id);
        return Err(AppError::Invalid("pair request expired".into()));
    }
    if pending.short_code != short_code {
        return Err(AppError::Invalid("pairing code mismatch".into()));
    }
    let peer = trust_discovered_device(state, device_id).await?;
    state.set_trusted_peer(peer).await?;
    send_wire_to_peer(
        state,
        device_id,
        WireMessage::PairDecision(PairDecision {
            request_id: pending.request_id,
            approved: true,
        }),
    )
    .await?;
    Ok(())
}

pub async fn reject_pair(state: &ManagedState, device_id: Uuid) -> AppResult<()> {
    let pending = state
        .0
        .pending_pairs
        .write()
        .await
        .remove(&device_id)
        .ok_or_else(|| AppError::Invalid("pair request not found".into()))?;
    let _ = send_wire_to_peer(
        state,
        device_id,
        WireMessage::PairDecision(PairDecision {
            request_id: pending.request_id,
            approved: false,
        }),
    )
    .await;
    Ok(())
}

pub async fn send_wire_to_peer(state: &ManagedState, device_id: Uuid, message: WireMessage) -> AppResult<()> {
    let peer = state
        .0
        .connected_peers
        .read()
        .await
        .get(&device_id)
        .cloned()
        .ok_or_else(|| AppError::Invalid("device is not connected".into()))?;
    send_wire_message(&peer.connection, &message).await
}

pub async fn trust_discovered_device(state: &ManagedState, device_id: Uuid) -> AppResult<TrustedPeer> {
    let discovered = state.0.discovered_peers.read().await;
    let peer = discovered
        .get(&device_id)
        .cloned()
        .ok_or_else(|| AppError::Invalid("device not found".into()))?;
    Ok(TrustedPeer {
        device_id: peer.device_id,
        device_name: peer.device_name,
        public_key: peer.public_key,
    })
}

async fn send_wire_message(connection: &quinn::Connection, message: &WireMessage) -> AppResult<()> {
    let (mut send, _recv) = connection
        .open_bi()
        .await
        .map_err(|error| AppError::Invalid(error.to_string()))?;
    write_stream_envelope(&mut send, &StreamEnvelope::Control { message: message.clone() }).await?;
    send.finish().map_err(|error| AppError::Invalid(error.to_string()))?;
    Ok(())
}

async fn write_stream_envelope(send: &mut SendStream, envelope: &StreamEnvelope) -> AppResult<()> {
    let bytes = serde_json::to_vec(envelope)?;
    let len = u32::try_from(bytes.len())
        .map_err(|_| AppError::Invalid("stream envelope too large".into()))?
        .to_be_bytes();
    send.write_all(&len)
        .await
        .map_err(|error| AppError::Invalid(error.to_string()))?;
    send.write_all(&bytes)
        .await
        .map_err(|error| AppError::Invalid(error.to_string()))?;
    Ok(())
}

async fn write_json<T: Serialize>(send: &mut SendStream, value: &T) -> AppResult<()> {
    let bytes = serde_json::to_vec(value)?;
    send.write_all(&bytes)
        .await
        .map_err(|error| AppError::Invalid(error.to_string()))?;
    send.finish().map_err(|error| AppError::Invalid(error.to_string()))?;
    Ok(())
}

async fn read_stream_envelope(recv: &mut RecvStream) -> AppResult<StreamEnvelope> {
    let mut len = [0u8; 4];
    recv.read_exact(&mut len)
        .await
        .map_err(|error| AppError::Invalid(error.to_string()))?;
    let header_len = u32::from_be_bytes(len) as usize;
    let mut header = vec![0u8; header_len];
    recv.read_exact(&mut header)
        .await
        .map_err(|error| AppError::Invalid(error.to_string()))?;
    serde_json::from_slice(&header).map_err(AppError::from)
}

async fn read_json<T: DeserializeOwned>(recv: &mut RecvStream) -> AppResult<T> {
    let frame = recv
        .read_to_end(MAX_FRAME_SIZE)
        .await
        .map_err(|error| AppError::Invalid(error.to_string()))?;
    serde_json::from_slice(&frame).map_err(AppError::from)
}

async fn send_file_transfer(
    connection: &quinn::Connection,
    session_key: &[u8; 32],
    payload: &ClipboardPayload,
    local_files: &[LocalFileSource],
) -> AppResult<()> {
    let crate::sync::protocol::ClipboardContent::Files { bundle } = &payload.content else {
        return Ok(());
    };

    let manifest_bytes = serde_json::to_vec(bundle)?;
    let (nonce, ciphertext) = crypto::encrypt(session_key, &manifest_bytes)?;
    let envelope = StreamEnvelope::FileTransfer {
        manifest: crate::sync::protocol::CipherPacket { nonce, ciphertext },
    };
    let (mut send, _recv) = connection
        .open_bi()
        .await
        .map_err(|error| AppError::Invalid(error.to_string()))?;
    write_stream_envelope(&mut send, &envelope).await?;

    for source in local_files {
        let mut file = tokio::fs::File::open(&source.source_path)
            .await
            .map_err(AppError::from)?;
        let mut remaining = source.byte_len;
        let mut buffer = vec![0u8; FILE_CHUNK_SIZE];
        while remaining > 0 {
            let read = file
                .read(&mut buffer)
                .await
                .map_err(AppError::from)?;
            if read == 0 {
                return Err(AppError::Invalid("unexpected eof while reading source file".into()));
            }
            send.write_all(&buffer[..read])
                .await
                .map_err(|error| AppError::Invalid(error.to_string()))?;
            remaining = remaining.saturating_sub(read as u64);
        }
    }

    send.finish().map_err(|error| AppError::Invalid(error.to_string()))?;
    Ok(())
}

async fn receive_file_transfer(
    state: ManagedState,
    remote_name: String,
    bundle: FileBundle,
    recv: &mut RecvStream,
) -> AppResult<()> {
    let root = clipboard::temp_bundle_dir(Some(&bundle.transfer_id.to_string()), &bundle.transfer_id.to_string())?;
    for file in &bundle.files {
        let path = clipboard::target_bundle_path(&root, &file.relative_path);
        let mut target = tokio::fs::File::create(&path).await.map_err(AppError::from)?;
        let mut remaining = file.byte_len;
        let mut buffer = vec![0u8; FILE_CHUNK_SIZE];
        while remaining > 0 {
            let limit = remaining.min(FILE_CHUNK_SIZE as u64) as usize;
            let read = recv
                .read(&mut buffer[..limit])
                .await
                .map_err(|error| AppError::Invalid(error.to_string()))?;
            let Some(read) = read else {
                return Err(AppError::Invalid("unexpected eof while receiving file stream".into()));
            };
            tokio::io::AsyncWriteExt::write_all(&mut target, &buffer[..read])
                .await
                .map_err(AppError::from)?;
            remaining = remaining.saturating_sub(read as u64);
        }
    }
    state
        .log(
            "INFO",
            format!("已接收来自 {remote_name} 的文件流，共 {} 个文件", bundle.files.len()),
        )
        .await;
    Ok(())
}

async fn wait_for_received_files(bundle: &FileBundle) -> AppResult<()> {
    let root = clipboard::temp_bundle_dir(Some(&bundle.transfer_id.to_string()), &bundle.transfer_id.to_string())?;
    for _ in 0..120 {
        let ready = bundle.files.iter().all(|file| {
            let path = clipboard::target_bundle_path(&root, &file.relative_path);
            std::fs::metadata(path)
                .map(|meta| meta.len() >= file.byte_len)
                .unwrap_or(false)
        });
        if ready {
            return Ok(());
        }
        sleep(Duration::from_millis(250)).await;
    }
    Err(AppError::Invalid("file transfer timed out".into()))
}

async fn run_housekeeping_loop(state: ManagedState) {
    let mut ticker = interval(Duration::from_secs(5));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        ticker.tick().await;
        let now = now_ms();

        let expired_pairs = {
            let pending = state.0.pending_pairs.read().await;
            pending
                .iter()
                .filter_map(|(device_id, pair)| {
                    (pair.expires_at_ms <= now).then_some((*device_id, pair.device_name.clone()))
                })
                .collect::<Vec<_>>()
        };

        for (device_id, device_name) in expired_pairs {
            state.0.pending_pairs.write().await.remove(&device_id);
            state
                .log("INFO", format!("设备 {device_name} 的配对请求已过期"))
                .await;
            if !state.0.trusted_peers.read().await.contains_key(&device_id) {
                if let Some(peer) = state.0.connected_peers.write().await.remove(&device_id) {
                    peer.connection.close(0u32.into(), b"pair request expired");
                }
            }
        }

        let stale_ids = {
            let discovered = state.0.discovered_peers.read().await;
            discovered
                .iter()
                .filter_map(|(device_id, peer)| {
                    (now.saturating_sub(peer.last_seen_ms) > DISCOVERY_TTL_MS).then_some(*device_id)
                })
                .collect::<HashSet<_>>()
        };

        if !stale_ids.is_empty() {
            let mut discovered = state.0.discovered_peers.write().await;
            for device_id in stale_ids {
                discovered.remove(&device_id);
            }
        }
    }
}
