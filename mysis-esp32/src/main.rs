mod chip;
mod memory_nvs;
mod mqtt_transport;
mod tools;

use esp_idf_hal::gpio::IOPin;
use esp_idf_hal::prelude::Peripherals;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::log::EspLogger;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::wifi::*;
use memory_nvs::NvsMemory;
use mqtt_transport::EspMqttTransport;
use mysis_core::agent::{run_agent_loop, AgentConfig};
use mysis_core::tool::Tool;
use std::sync::{Arc, Mutex};
use tools::gpio::GpioWriteTool;
use tools::memory::{MemoryDeleteTool, MemoryListTool, MemoryRecallTool, MemoryStoreTool};

// MVP 硬编码配置，Phase 2 将改为 NVS + build.rs
const WIFI_SSID: &str = "YOUR_SSID";
const WIFI_PASS: &str = "YOUR_PASS";
const MQTT_BROKER: &str = "mqtt://192.168.1.100:1883";
const DEVICE_ID: &str = "mysis-dev-01";

fn main() {
    EspLogger::initialize_default();
    log::info!("Mysis Agent starting...");

    let peripherals = Peripherals::take().unwrap();
    let sysloop = EspSystemEventLoop::take().unwrap();
    let nvs = EspDefaultNvsPartition::take().unwrap();

    // 初始化 NVS 记忆（L2 层）
    let nvs_memory = Arc::new(Mutex::new(
        NvsMemory::new(nvs.clone()).expect("failed to init NVS memory"),
    ));

    // 从 NVS 加载已有偏好
    let preferences = {
        let mem = nvs_memory.lock().unwrap();
        mem.load_all_preferences().unwrap_or_default()
    };
    log::info!("loaded {} preferences from NVS", preferences.len());

    // WiFi 连接
    let mut wifi = BlockingWifi::wrap(
        EspWifi::new(peripherals.modem, sysloop.clone(), Some(nvs)).unwrap(),
        sysloop.clone(),
    )
    .unwrap();
    connect_wifi(&mut wifi);
    log::info!("WiFi connected");

    // 初始化工具（使用芯片默认 GPIO 引脚）
    let default_pin = {
        #[cfg(feature = "esp32s3")]
        { peripherals.pins.gpio13.downgrade() }
        #[cfg(feature = "esp32c3")]
        { peripherals.pins.gpio4.downgrade() }
        #[cfg(feature = "esp32c6")]
        { peripherals.pins.gpio4.downgrade() }
    };
    let gpio_write = GpioWriteTool::new("living_room_light", default_pin).unwrap();
    let memory_store_tool = MemoryStoreTool::new(nvs_memory.clone());
    let memory_recall_tool = MemoryRecallTool::new(nvs_memory.clone());
    let memory_list_tool = MemoryListTool::new(nvs_memory.clone());
    let memory_delete_tool = MemoryDeleteTool::new(nvs_memory.clone());

    let mut all_tools: Vec<Box<dyn Tool>> = vec![
        Box::new(gpio_write),
        Box::new(memory_store_tool),
        Box::new(memory_recall_tool),
        Box::new(memory_list_tool),
        Box::new(memory_delete_tool),
    ];

    // MQTT 连接
    let mut transport = EspMqttTransport::new(MQTT_BROKER, DEVICE_ID).unwrap();
    log::info!("MQTT connected to {MQTT_BROKER}");

    // 冷启动恢复：请求 Bridge 同步长期记忆
    log::info!("requesting memory sync from Bridge...");
    if let Ok(sync) = transport.recv_memory_sync(5) {
        log::info!(
            "memory sync: {} preferences, summary: {}",
            sync.preferences.len(),
            &sync.summary
        );
    } else {
        log::info!("no memory sync response (Bridge may be offline)");
    }

    // 构建带记忆上下文的 system prompt
    let config = AgentConfig {
        device_id: DEVICE_ID.into(),
        chip_model: chip::CHIP_MODEL.into(),
        max_iterations: 5,
        llm_timeout_secs: 30,
        history_max_rounds: chip::MAX_HISTORY_ROUNDS,
        system_prompt: build_system_prompt(DEVICE_ID, &preferences),
    };

    // 主循环：等待 MQTT command，执行 agent 循环
    log::info!("Mysis Agent ready, waiting for commands...");
    loop {
        // MVP: 简单轮询检查命令主题
        // 后续改为事件驱动
        std::thread::sleep(std::time::Duration::from_millis(500));
        // TODO: 从 MQTT command 主题读取命令并触发 agent loop
        // 这部分在端到端集成时完善
    }
}

fn build_system_prompt(device_id: &str, preferences: &[(String, String)]) -> String {
    let mut prompt = format!(
        "你是 Mysis，一个运行在 ESP32 上的智能家居控制助手。\n设备 ID：{device_id}\n"
    );

    if !preferences.is_empty() {
        prompt.push_str("\n## 用户偏好\n");
        for (key, value) in preferences {
            prompt.push_str(&format!("- {key} = {value}\n"));
        }
    }

    prompt.push_str("\n## 可用工具\n");
    prompt.push_str("- gpio_write_living_room_light: 控制客厅灯\n");
    prompt.push_str("- memory_store: 记住用户偏好\n");
    prompt.push_str("- memory_recall: 查询已记住的信息\n");
    prompt.push_str("- memory_list: 列出指定分类下的所有记忆\n");
    prompt.push_str("- memory_delete: 删除一条记忆\n");

    prompt.push_str("\n## 规则\n");
    prompt.push_str("- 执行操作前确认安全，操作后报告结果。\n");
    prompt.push_str("- 当你发现用户的新偏好时，使用 memory_store 工具记住它。\n");
    prompt.push_str("- 当你需要查询历史信息时，使用 memory_recall 工具搜索。\n");
    prompt.push_str("- 当用户要求查看所有记忆时，使用 memory_list 工具。\n");
    prompt.push_str("- 当用户要求遗忘某条信息时，使用 memory_delete 工具。\n");
    prompt
}

fn connect_wifi(wifi: &mut BlockingWifi<EspWifi<'static>>) {
    wifi.set_configuration(&Configuration::Client(ClientConfiguration {
        ssid: WIFI_SSID.try_into().unwrap(),
        password: WIFI_PASS.try_into().unwrap(),
        ..Default::default()
    }))
    .unwrap();
    wifi.start().unwrap();
    wifi.connect().unwrap();
    wifi.wait_netif_up().unwrap();
}
