use esp_idf_hal::gpio::{AnyIOPin, IOPin, Input, Output, PinDriver};
use mysis_core::error::ToolError;
use mysis_core::tool::Tool;

pub struct GpioWriteTool {
    tool_name: String,
    pin_alias: String,
    driver: PinDriver<'static, AnyIOPin, Output>,
}

impl GpioWriteTool {
    pub fn new(alias: &str, pin: AnyIOPin) -> Result<Self, ToolError> {
        let driver = PinDriver::output(pin)
            .map_err(|e| ToolError::Unavailable(format!("failed to init GPIO output: {e}")))?;
        Ok(Self {
            tool_name: format!("gpio_write_{alias}"),
            pin_alias: alias.to_string(),
            driver,
        })
    }
}

impl Tool for GpioWriteTool {
    fn name(&self) -> &str {
        &self.tool_name
    }

    fn description(&self) -> &str {
        "控制 GPIO 引脚输出（高/低电平），用于开关灯、继电器等"
    }

    fn parameters_schema(&self) -> &str {
        r#"{"type":"object","properties":{"pin":{"type":"string","description":"引脚别名"},"value":{"type":"boolean","description":"true=高电平(开), false=低电平(关)"}},"required":["pin","value"]}"#
    }

    fn execute(&mut self, params: &str) -> Result<String, ToolError> {
        let v: serde_json::Value =
            serde_json::from_str(params).map_err(|e| ToolError::InvalidParams(e.to_string()))?;

        let pin = v["pin"]
            .as_str()
            .ok_or_else(|| ToolError::InvalidParams("missing 'pin'".into()))?;

        if pin != self.pin_alias {
            return Err(ToolError::InvalidParams(format!(
                "pin '{pin}' does not match this tool's pin '{}'",
                self.pin_alias
            )));
        }

        let value = v["value"]
            .as_bool()
            .ok_or_else(|| ToolError::InvalidParams("missing 'value'".into()))?;

        if value {
            self.driver
                .set_high()
                .map_err(|e| ToolError::ExecutionFailed(format!("set_high failed: {e}")))?;
        } else {
            self.driver
                .set_low()
                .map_err(|e| ToolError::ExecutionFailed(format!("set_low failed: {e}")))?;
        }

        Ok(format!(
            r#"{{"success":true,"pin":"{}","value":{}}}"#,
            self.pin_alias, value
        ))
    }
}

pub struct GpioReadTool {
    tool_name: String,
    pin_alias: String,
    driver: PinDriver<'static, AnyIOPin, Input>,
}

impl GpioReadTool {
    pub fn new(alias: &str, pin: AnyIOPin) -> Result<Self, ToolError> {
        let driver = PinDriver::input(pin)
            .map_err(|e| ToolError::Unavailable(format!("failed to init GPIO input: {e}")))?;
        Ok(Self {
            tool_name: format!("gpio_read_{alias}"),
            pin_alias: alias.to_string(),
            driver,
        })
    }
}

impl Tool for GpioReadTool {
    fn name(&self) -> &str {
        &self.tool_name
    }

    fn description(&self) -> &str {
        "读取 GPIO 引脚输入状态（高/低电平），用于检测按钮、开关状态"
    }

    fn parameters_schema(&self) -> &str {
        r#"{"type":"object","properties":{"pin":{"type":"string","description":"引脚别名"}},"required":["pin"]}"#
    }

    fn execute(&mut self, params: &str) -> Result<String, ToolError> {
        let v: serde_json::Value =
            serde_json::from_str(params).map_err(|e| ToolError::InvalidParams(e.to_string()))?;

        let pin = v["pin"]
            .as_str()
            .ok_or_else(|| ToolError::InvalidParams("missing 'pin'".into()))?;

        if pin != self.pin_alias {
            return Err(ToolError::InvalidParams(format!(
                "pin '{pin}' does not match this tool's pin '{}'",
                self.pin_alias
            )));
        }

        let value = self.driver.is_high();
        Ok(format!(
            r#"{{"success":true,"pin":"{}","value":{}}}"#,
            self.pin_alias, value
        ))
    }
}
