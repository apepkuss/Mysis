mod config;
mod device_manager;
mod embedder;
mod forwarder;
mod memory_handler;
mod memory_store;
mod scheduler;
mod time_service;

use crate::config::BridgeConfig;
use crate::device_manager::DeviceManager;
use crate::forwarder::LlmForwarder;
use crate::memory_store::SqliteMemoryStore;
use crate::scheduler::Scheduler;
use crate::time_service::TimeService;
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

    let recall_top_k = config.memory.recall_top_k;
    let similarity_threshold = config.memory.similarity_threshold;
    let forwarder = Arc::new(LlmForwarder::new(config.llm));
    let device_manager = Arc::new(Mutex::new(DeviceManager::new(Duration::from_secs(
        config.devices.heartbeat_timeout_secs,
    ))));

    // 初始化时间服务和调度器
    let time_service = Arc::new(Mutex::new(TimeService::new()));
    let scheduler = Arc::new(Mutex::new(Scheduler::new()));

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

    // 初始化嵌入模型（可选，降级为纯关键词匹配）
    let embedder: Arc<Option<Mutex<embedder::Embedder>>> =
        if let Some(ref model_dir) = config.memory.embedding_model_dir {
            match embedder::Embedder::new(model_dir, config.memory.embedding_dim) {
                Ok(e) => {
                    tracing::info!("embedding model loaded from {model_dir}");
                    Arc::new(Some(Mutex::new(e)))
                }
                Err(e) => {
                    tracing::warn!(
                        "embedding model not available: {e}. Falling back to keyword search. \
                         Run scripts/download_model.sh to download the model."
                    );
                    Arc::new(None)
                }
            }
        } else {
            tracing::info!("embedding model not configured, using keyword search only");
            Arc::new(None)
        };

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
    client.subscribe("mysis/+/time/#", QoS::AtLeastOnce).await?;
    client.subscribe("mysis/+/cron/#", QoS::AtLeastOnce).await?;
    tracing::info!("subscribed to MQTT topics");

    // 启动 cron tick 循环（每 30 秒检查一次到期任务）
    {
        let scheduler = scheduler.clone();
        let client = client.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(30)).await;
                let due_jobs = {
                    let mut sched = scheduler.lock().await;
                    sched.check_due_jobs()
                };
                for (device_id, action) in due_jobs {
                    let cmd = Command {
                        id: format!("cron-{}", chrono::Utc::now().timestamp()),
                        action: action.clone(),
                        tool: String::new(),
                        arguments: serde_json::Value::Null,
                    };
                    if let Ok(payload) = serde_json::to_vec(&cmd) {
                        let topic = Topics::command(&device_id);
                        if let Err(e) = client
                            .publish(&topic, QoS::AtLeastOnce, false, payload)
                            .await
                        {
                            tracing::error!("cron dispatch failed for {device_id}: {e}");
                        } else {
                            tracing::info!("cron dispatched to {device_id}: {action}");
                        }
                    }
                }
            }
        });
    }

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

                let time_service = time_service.clone();
                let scheduler = scheduler.clone();
                let embedder = embedder.clone();

                tokio::spawn(async move {
                    if let Err(e) = handle_message(
                        &topic,
                        &payload,
                        &client,
                        &forwarder,
                        &device_manager,
                        &memory_store,
                        &time_service,
                        &scheduler,
                        &embedder,
                        recall_top_k,
                        similarity_threshold,
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

#[allow(clippy::too_many_arguments)]
async fn handle_message(
    topic: &str,
    payload: &[u8],
    client: &AsyncClient,
    forwarder: &LlmForwarder,
    device_manager: &Arc<Mutex<DeviceManager>>,
    memory_store: &Arc<Mutex<SqliteMemoryStore>>,
    time_service: &Arc<Mutex<TimeService>>,
    scheduler: &Arc<Mutex<Scheduler>>,
    embedder: &Arc<Option<Mutex<embedder::Embedder>>>,
    recall_top_k: usize,
    similarity_threshold: f32,
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
        let memory_id = store
            .store_memory(device_id, &req.category, &req.content, req.metadata.clone())
            .map_err(|e| format!("store memory failed: {e}"))?;

        // 生成嵌入向量（如果模型可用）
        if let Some(ref emb_mutex) = **embedder {
            let mut emb = emb_mutex.lock().await;
            match emb.embed(&req.content) {
                Ok(vector) => {
                    store.store_embedding(memory_id, &vector)?;
                    tracing::debug!("stored embedding for memory {memory_id}");
                }
                Err(e) => {
                    tracing::warn!("embedding failed for memory {memory_id}: {e}");
                }
            }
        }

        tracing::info!("stored memory for {device_id}: {}", req.category);
    } else if topic.ends_with("/memory/recall") {
        let req: MemoryRecallRequest = serde_json::from_slice(payload)
            .map_err(|e| format!("invalid memory recall request JSON: {e}"))?;

        let store = memory_store.lock().await;

        // 使用混合召回（向量 + 关键词）
        let query_vector = if let Some(ref emb_mutex) = **embedder {
            let mut emb = emb_mutex.lock().await;
            emb.embed(&req.query).ok()
        } else {
            None
        };

        let memories = store.hybrid_recall(
            device_id,
            &req.query,
            query_vector.as_deref(),
            recall_top_k,
            similarity_threshold,
        )?;

        let result = MemoryRecallResult {
            id: req.id.clone(),
            memories,
        };
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
    } else if topic.ends_with("/time/get") {
        let svc = time_service.lock().await;
        let result = svc.get_time(device_id);
        let resp_topic = format!("mysis/{device_id}/time/response");
        client
            .publish(
                &resp_topic,
                QoS::AtLeastOnce,
                false,
                result.to_json().into_bytes(),
            )
            .await
            .map_err(|e| format!("MQTT publish failed: {e}"))?;
        tracing::info!("sent time to {device_id}");
    } else if topic.ends_with("/time/set_timezone") {
        #[derive(serde::Deserialize)]
        struct TzReq {
            timezone: String,
        }
        let req: TzReq = serde_json::from_slice(payload)
            .map_err(|e| format!("invalid timezone request: {e}"))?;

        let mut svc = time_service.lock().await;
        svc.set_timezone(device_id, &req.timezone)?;

        // 同步给调度器
        let mut sched = scheduler.lock().await;
        sched.set_device_timezone(device_id, &req.timezone);

        let resp_topic = format!("mysis/{device_id}/time/response");
        let resp = format!(r#"{{"success":true,"timezone":"{}"}}"#, req.timezone);
        client
            .publish(&resp_topic, QoS::AtLeastOnce, false, resp.into_bytes())
            .await
            .map_err(|e| format!("MQTT publish failed: {e}"))?;
        tracing::info!("set timezone for {device_id}: {}", req.timezone);
    } else if topic.ends_with("/time/get_timezone") {
        let svc = time_service.lock().await;
        let tz = svc.get_timezone(device_id);
        let resp_topic = format!("mysis/{device_id}/time/response");
        let resp = format!(r#"{{"timezone":"{tz}"}}"#);
        client
            .publish(&resp_topic, QoS::AtLeastOnce, false, resp.into_bytes())
            .await
            .map_err(|e| format!("MQTT publish failed: {e}"))?;
    } else if topic.ends_with("/cron/set") {
        #[derive(serde::Deserialize)]
        struct CronSetReq {
            #[serde(flatten)]
            cron_type: scheduler::CronType,
            action: String,
        }
        let req: CronSetReq = serde_json::from_slice(payload)
            .map_err(|e| format!("invalid cron_set request: {e}"))?;

        let mut sched = scheduler.lock().await;
        let job = sched.create_job(device_id, req.cron_type, &req.action);
        let resp = serde_json::to_vec(job).map_err(|e| format!("serialize failed: {e}"))?;

        let resp_topic = format!("mysis/{device_id}/cron/response");
        client
            .publish(&resp_topic, QoS::AtLeastOnce, false, resp)
            .await
            .map_err(|e| format!("MQTT publish failed: {e}"))?;
        tracing::info!("created cron job {} for {device_id}", job.id);
    } else if topic.ends_with("/cron/list") {
        let sched = scheduler.lock().await;
        let jobs = sched.list_jobs(device_id);
        let resp = serde_json::to_vec(&jobs).map_err(|e| format!("serialize failed: {e}"))?;

        let resp_topic = format!("mysis/{device_id}/cron/response");
        client
            .publish(&resp_topic, QoS::AtLeastOnce, false, resp)
            .await
            .map_err(|e| format!("MQTT publish failed: {e}"))?;
        tracing::info!("listed {} cron jobs for {device_id}", jobs.len());
    } else if topic.ends_with("/cron/delete") {
        #[derive(serde::Deserialize)]
        struct CronDeleteReq {
            id: u32,
        }
        let req: CronDeleteReq = serde_json::from_slice(payload)
            .map_err(|e| format!("invalid cron_delete request: {e}"))?;

        let mut sched = scheduler.lock().await;
        let deleted = sched.delete_job(device_id, req.id);
        let resp = format!(r#"{{"success":{deleted},"id":{}}}"#, req.id);

        let resp_topic = format!("mysis/{device_id}/cron/response");
        client
            .publish(&resp_topic, QoS::AtLeastOnce, false, resp.into_bytes())
            .await
            .map_err(|e| format!("MQTT publish failed: {e}"))?;
        tracing::info!("deleted cron job {} for {device_id}: {deleted}", req.id);
    }

    Ok(())
}
