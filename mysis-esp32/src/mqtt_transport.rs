use crate::chip;
use esp_idf_svc::mqtt::client::*;
use mysis_core::agent::Transport;
use mysis_core::protocol::*;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub struct EspMqttTransport {
    client: EspMqttClient<'static>,
    device_id: String,
    /// 接收 LLM 响应的缓冲（由 MQTT 回调写入）
    response_buf: Arc<Mutex<Option<LlmResponse>>>,
    /// 接收记忆同步响应的缓冲
    sync_buf: Arc<Mutex<Option<MemorySyncResponse>>>,
}

impl EspMqttTransport {
    pub fn new(broker_url: &str, device_id: &str) -> Result<Self, String> {
        let response_buf: Arc<Mutex<Option<LlmResponse>>> = Arc::new(Mutex::new(None));
        let sync_buf: Arc<Mutex<Option<MemorySyncResponse>>> = Arc::new(Mutex::new(None));
        let buf_clone = response_buf.clone();
        let sync_clone = sync_buf.clone();
        let resp_topic = Topics::llm_response(device_id);
        let sync_topic = Topics::memory_sync(device_id);

        let conf = MqttClientConfiguration {
            client_id: Some(device_id),
            buffer_size: chip::MQTT_RX_BUF_SIZE,
            out_buffer_size: chip::MQTT_TX_BUF_SIZE,
            ..Default::default()
        };

        let mut client = EspMqttClient::new_cb(broker_url, &conf, move |event| {
            if let Ok(Event::Received(msg)) = event {
                if let Some(topic) = msg.topic() {
                    if topic == resp_topic.as_str() {
                        if let Ok(resp) = serde_json::from_slice::<LlmResponse>(msg.data()) {
                            let mut buf = buf_clone.lock().unwrap();
                            *buf = Some(resp);
                        }
                    } else if topic == sync_topic.as_str() {
                        if let Ok(sync) = serde_json::from_slice::<MemorySyncResponse>(msg.data())
                        {
                            let mut buf = sync_clone.lock().unwrap();
                            *buf = Some(sync);
                        }
                    }
                }
            }
        })
        .map_err(|e| format!("MQTT connect failed: {e}"))?;

        // 订阅 LLM 响应、命令和记忆同步主题
        client
            .subscribe(&Topics::llm_response(device_id), QoS::AtLeastOnce)
            .map_err(|e| format!("subscribe failed: {e}"))?;
        client
            .subscribe(&Topics::command(device_id), QoS::AtLeastOnce)
            .map_err(|e| format!("subscribe failed: {e}"))?;
        client
            .subscribe(&Topics::memory_sync(device_id), QoS::AtLeastOnce)
            .map_err(|e| format!("subscribe failed: {e}"))?;
        client
            .subscribe(&Topics::memory_result(device_id), QoS::AtLeastOnce)
            .map_err(|e| format!("subscribe failed: {e}"))?;

        Ok(Self {
            client,
            device_id: device_id.to_string(),
            response_buf,
            sync_buf,
        })
    }

    /// 接收冷启动记忆同步响应（带超时）
    pub fn recv_memory_sync(
        &mut self,
        timeout_secs: u32,
    ) -> Result<MemorySyncResponse, String> {
        // 发送同步请求
        let topic = Topics::memory_recall(&self.device_id);
        let payload = serde_json::to_vec(&serde_json::json!({
            "id": format!("sync-{}", self.device_id),
            "action": "sync"
        }))
        .map_err(|e| format!("serialize failed: {e}"))?;
        self.client
            .enqueue(&topic, QoS::AtLeastOnce, false, &payload)
            .map_err(|e| format!("MQTT publish failed: {e}"))?;

        // 等待响应
        let deadline = std::time::Instant::now() + Duration::from_secs(timeout_secs as u64);
        loop {
            {
                let mut buf = self.sync_buf.lock().unwrap();
                if let Some(sync) = buf.take() {
                    return Ok(sync);
                }
            }
            if std::time::Instant::now() >= deadline {
                return Err("memory sync timeout".into());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

impl Transport for EspMqttTransport {
    fn send_llm_request(&mut self, req: &LlmRequest) -> Result<(), String> {
        // 发送前清空旧响应，避免竞态条件
        {
            let mut buf = self.response_buf.lock().unwrap();
            *buf = None;
        }

        let topic = Topics::llm_request(&self.device_id);
        let payload =
            serde_json::to_vec(req).map_err(|e| format!("serialize failed: {e}"))?;
        self.client
            .enqueue(&topic, QoS::AtLeastOnce, false, &payload)
            .map_err(|e| format!("MQTT publish failed: {e}"))?;
        Ok(())
    }

    fn recv_llm_response(&mut self, timeout_secs: u32) -> Result<LlmResponse, String> {
        let deadline = std::time::Instant::now() + Duration::from_secs(timeout_secs as u64);
        loop {
            {
                let mut buf = self.response_buf.lock().unwrap();
                if let Some(resp) = buf.take() {
                    return Ok(resp);
                }
            }
            if std::time::Instant::now() >= deadline {
                return Err("LLM response timeout".into());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}
