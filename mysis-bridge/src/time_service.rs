use chrono::{Local, Utc};
use chrono_tz::Tz;
use std::collections::HashMap;

/// 管理各设备的时区设置
pub struct TimeService {
    /// device_id -> timezone string (e.g. "Asia/Shanghai")
    timezones: HashMap<String, String>,
}

impl TimeService {
    pub fn new() -> Self {
        Self {
            timezones: HashMap::new(),
        }
    }

    /// 获取指定设备的当前时间
    pub fn get_time(&self, device_id: &str) -> TimeResult {
        let tz_str = self
            .timezones
            .get(device_id)
            .map(|s| s.as_str())
            .unwrap_or("UTC");

        if let Ok(tz) = tz_str.parse::<Tz>() {
            let now = Utc::now().with_timezone(&tz);
            TimeResult {
                datetime: now.format("%Y-%m-%d %H:%M:%S").to_string(),
                timezone: tz_str.to_string(),
                unix_timestamp: now.timestamp(),
            }
        } else {
            let now = Local::now();
            TimeResult {
                datetime: now.format("%Y-%m-%d %H:%M:%S").to_string(),
                timezone: "local".to_string(),
                unix_timestamp: now.timestamp(),
            }
        }
    }

    /// 设置设备时区
    pub fn set_timezone(&mut self, device_id: &str, timezone: &str) -> Result<(), String> {
        // 验证时区字符串有效
        timezone.parse::<Tz>().map_err(|_| {
            format!("invalid timezone: '{timezone}'. Use IANA format like 'Asia/Shanghai'")
        })?;
        self.timezones
            .insert(device_id.to_string(), timezone.to_string());
        Ok(())
    }

    /// 获取设备当前时区
    pub fn get_timezone(&self, device_id: &str) -> String {
        self.timezones
            .get(device_id)
            .cloned()
            .unwrap_or_else(|| "UTC".to_string())
    }
}

pub struct TimeResult {
    pub datetime: String,
    pub timezone: String,
    pub unix_timestamp: i64,
}

impl TimeResult {
    pub fn to_json(&self) -> String {
        format!(
            r#"{{"datetime":"{}","timezone":"{}","unix_timestamp":{}}}"#,
            self.datetime, self.timezone, self.unix_timestamp
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_timezone_is_utc() {
        let svc = TimeService::new();
        assert_eq!(svc.get_timezone("dev-01"), "UTC");
    }

    #[test]
    fn set_and_get_timezone() {
        let mut svc = TimeService::new();
        svc.set_timezone("dev-01", "Asia/Shanghai").unwrap();
        assert_eq!(svc.get_timezone("dev-01"), "Asia/Shanghai");
    }

    #[test]
    fn invalid_timezone_rejected() {
        let mut svc = TimeService::new();
        assert!(svc.set_timezone("dev-01", "Invalid/Zone").is_err());
    }

    #[test]
    fn get_time_returns_valid_result() {
        let mut svc = TimeService::new();
        svc.set_timezone("dev-01", "Asia/Shanghai").unwrap();
        let result = svc.get_time("dev-01");
        assert_eq!(result.timezone, "Asia/Shanghai");
        assert!(result.unix_timestamp > 0);
    }
}
