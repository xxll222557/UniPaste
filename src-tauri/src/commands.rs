use std::sync::atomic::Ordering;

use tauri::State;
use uuid::Uuid;

use crate::{
    app_state::ManagedState,
    sync::{engine, protocol::Snapshot},
};

#[tauri::command]
pub async fn get_snapshot(state: State<'_, ManagedState>) -> Result<Snapshot, String> {
    Ok(state.snapshot().await)
}

#[tauri::command]
pub async fn request_pair(device_id: String, state: State<'_, ManagedState>) -> Result<(), String> {
    let device_id = Uuid::parse_str(&device_id).map_err(|error| error.to_string())?;
    let pair = engine::create_outbound_pair(&state, device_id)
        .await
        .map_err(|error| error.to_string())?;
    state
        .log(
            "INFO",
            format!("已发起配对，请在两台设备上核对短码 {}", pair.short_code),
        )
        .await;
    let _ = engine::send_wire_to_peer(
        &state,
        device_id,
        crate::sync::protocol::WireMessage::PairRequest(crate::sync::protocol::PairRequest {
            request_id: pair.request_id,
            short_code: pair.short_code.clone(),
        }),
    )
    .await;
    Ok(())
}

#[tauri::command]
pub async fn approve_pair(
    device_id: String,
    short_code: String,
    state: State<'_, ManagedState>,
) -> Result<(), String> {
    let device_id = Uuid::parse_str(&device_id).map_err(|error| error.to_string())?;
    let short_code = short_code.trim().to_string();
    if short_code.len() != 6 || !short_code.chars().all(|char| char.is_ascii_digit()) {
        return Err("请输入 6 位数字配对码".to_string());
    }
    engine::approve_pair(&state, device_id, &short_code)
        .await
        .map_err(|error| error.to_string())?;
    state.log("INFO", format!("设备 {device_id} 已确认并加入信任列表")).await;
    Ok(())
}

#[tauri::command]
pub async fn reject_pair(device_id: String, state: State<'_, ManagedState>) -> Result<(), String> {
    let device_id = Uuid::parse_str(&device_id).map_err(|error| error.to_string())?;
    engine::reject_pair(&state, device_id)
        .await
        .map_err(|error| error.to_string())?;
    state.log("INFO", format!("已拒绝设备 {device_id} 的配对请求")).await;
    Ok(())
}

#[tauri::command]
pub async fn remove_trusted_device(device_id: String, state: State<'_, ManagedState>) -> Result<(), String> {
    let device_id = Uuid::parse_str(&device_id).map_err(|error| error.to_string())?;
    state
        .remove_trusted_peer(device_id)
        .await
        .map_err(|error| error.to_string())?;
    state.log("INFO", format!("设备 {device_id} 已移出信任列表")).await;
    Ok(())
}

#[tauri::command]
pub async fn set_sync_enabled(enabled: bool, state: State<'_, ManagedState>) -> Result<(), String> {
    state.0.sync_enabled.store(enabled, Ordering::Relaxed);
    state.persist().await.map_err(|error| error.to_string())?;
    state
        .log("INFO", if enabled { "剪贴板同步已开启" } else { "剪贴板同步已暂停" })
        .await;
    Ok(())
}

#[tauri::command]
pub async fn update_device_name(device_name: String, state: State<'_, ManagedState>) -> Result<(), String> {
    let device_name = device_name.trim().to_string();
    if device_name.is_empty() {
        return Err("设备名称不能为空".to_string());
    }
    if device_name.chars().count() > 32 {
        return Err("设备名称不能超过 32 个字符".to_string());
    }
    state
        .update_device_name(device_name.clone())
        .await
        .map_err(|error| error.to_string())?;
    state
        .log("INFO", format!("设备名称已更新为 {device_name}"))
        .await;
    Ok(())
}

#[tauri::command]
pub async fn clear_history(state: State<'_, ManagedState>) -> Result<(), String> {
    state.clear_history().await.map_err(|error| error.to_string())?;
    state.log("INFO", "已清空同步历史").await;
    Ok(())
}

#[tauri::command]
pub async fn update_sync_preferences(
    sync_html: bool,
    sync_images: bool,
    sync_files: bool,
    state: State<'_, ManagedState>,
) -> Result<(), String> {
    state
        .update_sync_preferences(sync_html, sync_images, sync_files)
        .await
        .map_err(|error| error.to_string())?;
    state
        .log(
            "INFO",
            format!(
                "同步偏好已更新: HTML={} 图片={} 文件={}",
                if sync_html { "开" } else { "关" },
                if sync_images { "开" } else { "关" },
                if sync_files { "开" } else { "关" }
            ),
        )
        .await;
    Ok(())
}
