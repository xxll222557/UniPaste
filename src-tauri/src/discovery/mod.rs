use std::{collections::HashMap, time::Duration};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use ed25519_dalek::VerifyingKey;
use mdns_sd::{Receiver, ServiceDaemon, ServiceEvent, ServiceInfo};
use tokio::time::sleep;

use crate::{
    app_state::{now_ms, DiscoveredDevice, ManagedState},
    config::fingerprint,
};

const SERVICE_TYPE: &str = "_unipaste._udp.local.";

pub async fn run(state: ManagedState) {
    while state.0.quic_port.load(std::sync::atomic::Ordering::Relaxed) == 0 {
        sleep(Duration::from_millis(250)).await;
    }

    let mdns = match ServiceDaemon::new() {
        Ok(daemon) => daemon,
        Err(error) => {
            state.report_error(format!("mDNS daemon 启动失败: {error}")).await;
            return;
        }
    };

    if let Err(error) = register_service(&mdns, &state).await {
        state.report_error(format!("mDNS 服务注册失败: {error}")).await;
        return;
    }

    let receiver = match mdns.browse(SERVICE_TYPE) {
        Ok(receiver) => receiver,
        Err(error) => {
            state.report_error(format!("mDNS 浏览失败: {error}")).await;
            return;
        }
    };

    state.log("INFO", "mDNS 发现已启动").await;
    state.set_last_error(None).await;
    consume_events(state, receiver).await;
}

async fn register_service(mdns: &ServiceDaemon, state: &ManagedState) -> Result<(), String> {
    let identity = state.0.identity.lock().await.clone();
    let port = state.0.quic_port.load(std::sync::atomic::Ordering::Relaxed);
    let service_name = format!("unipaste-{}", identity.device_id);
    let hostname = format!("unipaste-{}.local.", identity.device_id);
    let properties: HashMap<String, String> = HashMap::from([
        ("id".into(), identity.device_id.to_string()),
        ("name".into(), identity.device_name.clone()),
        (
            "pk".into(),
            STANDARD.encode(identity.signing_key.verifying_key().to_bytes()),
        ),
        ("fp".into(), identity.fingerprint.clone()),
        ("proto".into(), "quic-v1".into()),
    ]);

    let service = ServiceInfo::new(SERVICE_TYPE, &service_name, &hostname, "", port, properties)
        .map_err(|error| error.to_string())?
        .enable_addr_auto();
    mdns.register(service).map_err(|error| error.to_string())
}

async fn consume_events(state: ManagedState, receiver: Receiver<ServiceEvent>) {
    while let Ok(event) = receiver.recv_async().await {
        match event {
            ServiceEvent::ServiceResolved(info) => {
                if let Err(error) = update_from_info(&state, info).await {
                    state.log("WARN", format!("忽略无效 mDNS 设备记录: {error}")).await;
                }
            }
            ServiceEvent::ServiceRemoved(_, fullname) => {
                state.log("INFO", format!("设备下线: {fullname}")).await;
            }
            _ => {}
        }
    }
}

async fn update_from_info(state: &ManagedState, info: ServiceInfo) -> Result<(), String> {
    let device_id = info
        .get_property_val_str("id")
        .ok_or_else(|| "missing device id".to_string())?
        .parse()
        .map_err(|error: uuid::Error| error.to_string())?;

    let local_device_id = state.0.identity.lock().await.device_id;
    if device_id == local_device_id {
        return Ok(());
    }

    let device_name = info
        .get_property_val_str("name")
        .ok_or_else(|| "missing device name".to_string())?
        .to_string();
    let public_key_b64 = info
        .get_property_val_str("pk")
        .ok_or_else(|| "missing device public key".to_string())?;
    let public_key_vec = STANDARD.decode(public_key_b64).map_err(|error| error.to_string())?;
    let public_key_bytes: [u8; 32] = public_key_vec
        .try_into()
        .map_err(|_| "bad public key length".to_string())?;
    let public_key = VerifyingKey::from_bytes(&public_key_bytes).map_err(|error| error.to_string())?;
    let address = info
        .get_addresses()
        .iter()
        .find(|ip| ip.is_ipv4())
        .or_else(|| info.get_addresses().iter().next())
        .ok_or_else(|| "missing service address".to_string())?
        .to_string();

    state.0.discovered_peers.write().await.insert(
        device_id,
        DiscoveredDevice {
            device_id,
            device_name,
            fingerprint: info
                .get_property_val_str("fp")
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| fingerprint(&public_key.to_bytes())),
            public_key,
            address,
            quic_port: info.get_port(),
            last_seen_ms: now_ms(),
        },
    );

    Ok(())
}
