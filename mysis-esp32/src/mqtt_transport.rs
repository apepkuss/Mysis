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
}

impl EspMqttTransport {
    pub fn new(broker_url: &str, device_id: &str) -> Result<Self, String> {
        let response_buf: Arc<Mutex<Option<LlmResponse>>> = Arc::new(Mutex::new(None));
        let buf_clone = response_buf.clone();
        let resp_topic = Topics::llm_response(device_id);

        let conf = MqttClientConfiguration {
            client_id: Some(device_id),
            ..Default::default()
        };

        let mut client = EspMqttClient::new_cb(broker_url, &conf, move |event| {
            if let Ok(Event::Received(msg)) = event {
                if msg.topic() == Some(resp_topic.as_str()) {
                    if let Ok(resp) = serde_json::from_slice::<LlmResponse>(msg.data()) {
                        let mut buf = buf_clone.lock().unwrap();
                        *buf = Some(resp);
                    }
                }
            }
        })
        .map_err(|e| format!("MQTT connect failed: {e}"))?;

        // 订阅 LLM 响应和命令主题
        client
            .subscribe(&Topics::llm_response(device_id), QoS::AtLeastOnce)
            .map_err(|e| format!("subscribe failed: {e}"))?;
        client
            .subscribe(&Topics::command(device_id), QoS::AtLeastOnce)
            .map_err(|e| format!("subscribe failed: {e}"))?;

        Ok(Self {
            client,
            device_id: device_id.to_string(),
            response_buf,
        })
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
