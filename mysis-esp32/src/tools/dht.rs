use esp_idf_hal::delay::Ets;
use esp_idf_hal::gpio::{AnyIOPin, IOPin, InputOutput, PinDriver};
use mysis_core::error::ToolError;
use mysis_core::tool::Tool;
use std::time::Duration;

/// DHT 传感器型号
#[derive(Clone, Copy)]
pub enum DhtModel {
    Dht11,
    Dht22,
}

/// DHT 温湿度传感器读取工具
pub struct DhtReadTool {
    tool_name: String,
    pin_alias: String,
    model: DhtModel,
    driver: PinDriver<'static, AnyIOPin, InputOutput>,
}

impl DhtReadTool {
    pub fn new(alias: &str, pin: AnyIOPin, model: DhtModel) -> Result<Self, ToolError> {
        let driver = PinDriver::input_output(pin)
            .map_err(|e| ToolError::Unavailable(format!("failed to init DHT pin: {e}")))?;
        Ok(Self {
            tool_name: format!("dht_read_{alias}"),
            pin_alias: alias.to_string(),
            model,
            driver,
        })
    }

    /// 读取 DHT 传感器原始 40-bit 数据
    fn read_raw(&mut self) -> Result<[u8; 5], ToolError> {
        // 1. 主机拉低启动信号
        self.driver
            .set_low()
            .map_err(|e| ToolError::ExecutionFailed(format!("set_low failed: {e}")))?;

        match self.model {
            DhtModel::Dht11 => Ets::delay_ms(20), // DHT11 需要 ≥18ms
            DhtModel::Dht22 => Ets::delay_ms(2),  // DHT22 需要 ≥1ms
        }

        // 2. 主机释放总线，等待传感器响应
        self.driver
            .set_high()
            .map_err(|e| ToolError::ExecutionFailed(format!("set_high failed: {e}")))?;
        Ets::delay_us(30);

        // 3. 等待传感器拉低（响应信号）
        self.wait_for_level(false, 100)?;
        // 4. 等待传感器拉高
        self.wait_for_level(true, 100)?;
        // 5. 等待传感器再次拉低（数据开始）
        self.wait_for_level(false, 100)?;

        // 6. 读取 40 bit 数据
        let mut data = [0u8; 5];
        for byte in &mut data {
            for bit in (0..8).rev() {
                // 等待拉高（数据位开始）
                self.wait_for_level(true, 80)?;
                // 测量高电平持续时间：>40us 为 1，<30us 为 0
                Ets::delay_us(35);
                if self.driver.is_high() {
                    *byte |= 1 << bit;
                    // 等待回到低电平
                    self.wait_for_level(false, 60)?;
                }
            }
        }

        // 7. 校验
        let checksum = data[0]
            .wrapping_add(data[1])
            .wrapping_add(data[2])
            .wrapping_add(data[3]);
        if checksum != data[4] {
            return Err(ToolError::ExecutionFailed(format!(
                "checksum mismatch: expected {}, got {}",
                data[4], checksum
            )));
        }

        Ok(data)
    }

    /// 等待引脚到达指定电平，超时返回错误
    fn wait_for_level(&self, high: bool, timeout_us: u32) -> Result<(), ToolError> {
        for _ in 0..timeout_us {
            if self.driver.is_high() == high {
                return Ok(());
            }
            Ets::delay_us(1);
        }
        Err(ToolError::ExecutionFailed(
            "DHT sensor timeout".to_string(),
        ))
    }

    /// 解析原始数据为温湿度
    fn parse_data(&self, data: [u8; 5]) -> (f32, f32) {
        match self.model {
            DhtModel::Dht11 => {
                let humidity = data[0] as f32 + data[1] as f32 * 0.1;
                let temperature = data[2] as f32 + (data[3] & 0x7F) as f32 * 0.1;
                let temperature = if data[3] & 0x80 != 0 {
                    -temperature
                } else {
                    temperature
                };
                (temperature, humidity)
            }
            DhtModel::Dht22 => {
                let humidity = ((data[0] as u16) << 8 | data[1] as u16) as f32 * 0.1;
                let raw_temp = ((data[2] as u16 & 0x7F) << 8 | data[3] as u16) as f32 * 0.1;
                let temperature = if data[2] & 0x80 != 0 {
                    -raw_temp
                } else {
                    raw_temp
                };
                (temperature, humidity)
            }
        }
    }
}

impl Tool for DhtReadTool {
    fn name(&self) -> &str {
        &self.tool_name
    }

    fn description(&self) -> &str {
        "读取 DHT 温湿度传感器数据，返回温度（摄氏度）和湿度（百分比）"
    }

    fn parameters_schema(&self) -> &str {
        r#"{"type":"object","properties":{"sensor":{"type":"string","description":"传感器别名"}},"required":["sensor"]}"#
    }

    fn execute(&mut self, params: &str) -> Result<String, ToolError> {
        let v: serde_json::Value =
            serde_json::from_str(params).map_err(|e| ToolError::InvalidParams(e.to_string()))?;

        let sensor = v["sensor"]
            .as_str()
            .ok_or_else(|| ToolError::InvalidParams("missing 'sensor'".into()))?;

        if sensor != self.pin_alias {
            return Err(ToolError::InvalidParams(format!(
                "sensor '{sensor}' does not match this tool's sensor '{}'",
                self.pin_alias
            )));
        }

        // 重试最多 3 次
        let mut last_err = String::new();
        for _ in 0..3 {
            match self.read_raw() {
                Ok(data) => {
                    let (temperature, humidity) = self.parse_data(data);
                    return Ok(format!(
                        r#"{{"success":true,"sensor":"{sensor}","temperature":{temperature:.1},"humidity":{humidity:.1},"unit":"celsius"}}"#
                    ));
                }
                Err(e) => {
                    last_err = e.to_string();
                    // DHT 传感器需要间隔至少 1 秒
                    std::thread::sleep(Duration::from_secs(1));
                }
            }
        }

        Err(ToolError::ExecutionFailed(format!(
            "DHT read failed after 3 retries: {last_err}"
        )))
    }
}
