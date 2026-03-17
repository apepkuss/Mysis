mod config;
mod device_manager;
mod forwarder;
mod memory_handler;
mod memory_store;

use crate::config::BridgeConfig;
use crate::device_manager::DeviceManager;
use crate::forwarder::LlmForwarder;
use crate::memory_handler::{handle_memory_recall, handle_memory_store};
use crate::memory_store::SqliteMemoryStore;
use mysis_core::protocol::*;

use clap::Parser;
use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

#[derive(Parser)]
#[command(name = "mysis-bridge", about = "Mysis MQTT-to-LLM bridge service")]
struct Cli {
    /// Path to config file
    #[arg(short, long, default_value = "mysis-bridge.toml")]
    config: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();
    let config = BridgeConfig::from_file(&cli.config)?;
    tracing::info!("loaded config from {}", cli.config);

    let forwarder = Arc::new(LlmForwarder::new(config.llm));
    let device_manager = Arc::new(Mutex::new(DeviceManager::new(Duration::from_secs(
        config.devices.heartbeat_timeout_secs,
    ))));

    // 初始化记忆存储
    let db_path = &config.memory.db_path;
    if let Some(parent) = std::path::Path::new(db_path).parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let memory_store = Arc::new(Mutex::new(
        SqliteMemoryStore::open(db_path)
            .map_err(|e| format!("failed to open memory database: {e}"))?,
    ));
    tracing::info!("memory store initialized at {db_path}");

    // MQTT 连接
    let mut mqtt_opts = MqttOptions::new(
        &config.mqtt.client_id,
        &config.mqtt.broker,
        config.mqtt.port,
    );
    mqtt_opts.set_keep_alive(Duration::from_secs(30));

    let (client, mut eventloop) = AsyncClient::new(mqtt_opts, 64);

    // 订阅所有设备的 LLM 请求、状态和记忆主题
    client
        .subscribe("mysis/+/llm/request", QoS::AtLeastOnce)
        .await?;
    client.subscribe("mysis/+/status", QoS::AtLeastOnce).await?;
    client
        .subscribe("mysis/+/memory/store", QoS::AtLeastOnce)
        .await?;
    client
        .subscribe("mysis/+/memory/recall", QoS::AtLeastOnce)
        .await?;
    tracing::info!("subscribed to MQTT topics");

    // 事件循环
    loop {
        match eventloop.poll().await {
            Ok(Event::Incoming(Packet::Publish(publish))) => {
                let topic = publish.topic.clone();
                let payload = publish.payload.to_vec();
                let client = client.clone();
                let forwarder = forwarder.clone();
                let device_manager = device_manager.clone();
                let memory_store = memory_store.clone();

                tokio::spawn(async move {
                    if let Err(e) = handle_message(
                        &topic,
                        &payload,
                        &client,
                        &forwarder,
                        &device_manager,
                        &memory_store,
                    )
                    .await
                    {
                        tracing::error!("error handling {topic}: {e}");
                    }
                });
            }
            Ok(_) => {}
            Err(e) => {
                tracing::error!("MQTT connection error: {e}");
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    }
}

async fn handle_message(
    topic: &str,
    payload: &[u8],
    client: &AsyncClient,
    forwarder: &LlmForwarder,
    device_manager: &Arc<Mutex<DeviceManager>>,
    memory_store: &Arc<Mutex<SqliteMemoryStore>>,
) -> Result<(), String> {
    let parts: Vec<&str> = topic.split('/').collect();
    // 预期格式: mysis/{device_id}/{type}/...
    if parts.len() < 3 || parts[0] != "mysis" {
        return Ok(());
    }
    let device_id = parts[1];

    if topic.ends_with("/llm/request") {
        // LLM 请求转发
        let req: LlmRequest = serde_json::from_slice(payload)
            .map_err(|e| format!("invalid LLM request JSON: {e}"))?;

        tracing::info!("forwarding LLM request {} from {device_id}", req.id);

        let resp = forwarder.forward(&req).await?;
        let resp_json =
            serde_json::to_vec(&resp).map_err(|e| format!("failed to serialize response: {e}"))?;

        let resp_topic = Topics::llm_response(device_id);
        client
            .publish(&resp_topic, QoS::AtLeastOnce, false, resp_json)
            .await
            .map_err(|e| format!("MQTT publish failed: {e}"))?;

        tracing::info!("sent LLM response to {resp_topic}");
    } else if topic.ends_with("/memory/store") {
        let req: MemoryStoreRequest = serde_json::from_slice(payload)
            .map_err(|e| format!("invalid memory store request JSON: {e}"))?;

        let mut store = memory_store.lock().await;
        handle_memory_store(&mut store, device_id, &req)?;
        tracing::info!("stored memory for {device_id}: {}", req.category);
    } else if topic.ends_with("/memory/recall") {
        let req: MemoryRecallRequest = serde_json::from_slice(payload)
            .map_err(|e| format!("invalid memory recall request JSON: {e}"))?;

        let store = memory_store.lock().await;
        let result = handle_memory_recall(&store, device_id, &req)?;
        let result_json = serde_json::to_vec(&result)
            .map_err(|e| format!("failed to serialize recall result: {e}"))?;

        let result_topic = Topics::memory_result(device_id);
        client
            .publish(&result_topic, QoS::AtLeastOnce, false, result_json)
            .await
            .map_err(|e| format!("MQTT publish failed: {e}"))?;

        tracing::info!(
            "recalled {} memories for {device_id}",
            result.memories.len()
        );
    } else if topic.ends_with("/status") {
        // 心跳处理
        let heartbeat: Heartbeat =
            serde_json::from_slice(payload).map_err(|e| format!("invalid heartbeat JSON: {e}"))?;

        let mut mgr = device_manager.lock().await;
        mgr.update_heartbeat(&heartbeat.device_id, &heartbeat.tools);
        tracing::debug!(
            "heartbeat from {}: rssi={}",
            heartbeat.device_id,
            heartbeat.wifi_rssi
        );
    }

    Ok(())
}
