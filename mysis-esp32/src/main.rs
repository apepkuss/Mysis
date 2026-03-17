mod mqtt_transport;
mod tools;

use esp_idf_hal::gpio::IOPin;
use esp_idf_hal::prelude::Peripherals;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::log::EspLogger;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::wifi::*;
use mqtt_transport::EspMqttTransport;
use mysis_core::agent::{run_agent_loop, AgentConfig};
use mysis_core::tool::Tool;
use tools::gpio::GpioWriteTool;

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

    // WiFi 连接
    let mut wifi = BlockingWifi::wrap(
        EspWifi::new(peripherals.modem, sysloop.clone(), Some(nvs)).unwrap(),
        sysloop.clone(),
    )
    .unwrap();
    connect_wifi(&mut wifi);
    log::info!("WiFi connected");

    // 初始化工具（GPIO 引脚分配）
    let gpio_write = GpioWriteTool::new(
        "living_room_light",
        peripherals.pins.gpio13.downgrade(),
    )
    .unwrap();

    let mut all_tools: Vec<Box<dyn Tool>> = vec![Box::new(gpio_write)];

    // MQTT 连接
    let mut transport = EspMqttTransport::new(MQTT_BROKER, DEVICE_ID).unwrap();
    log::info!("MQTT connected to {MQTT_BROKER}");

    let config = AgentConfig {
        device_id: DEVICE_ID.into(),
        chip_model: "esp32s3".into(),
        max_iterations: 5,
        llm_timeout_secs: 30,
        history_max_rounds: 10,
        system_prompt: format!(
            "你是 Mysis，一个运行在 ESP32 上的智能家居控制助手。\n\
             设备 ID：{}\n\
             可用工具：gpio_write_living_room_light\n\
             规则：执行操作前确认安全，操作后报告结果。",
            DEVICE_ID
        ),
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
