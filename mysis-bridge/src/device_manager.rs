use std::collections::HashMap;
use std::time::{Duration, Instant};

pub struct DeviceInfo {
    pub tools: Vec<String>,
    pub last_seen: Instant,
    pub online: bool,
}

pub struct DeviceManager {
    devices: HashMap<String, DeviceInfo>,
    timeout: Duration,
}

impl DeviceManager {
    pub fn new(timeout: Duration) -> Self {
        Self {
            devices: HashMap::new(),
            timeout,
        }
    }

    pub fn update_heartbeat(&mut self, device_id: &str, tools: &[String]) {
        let info = self
            .devices
            .entry(device_id.to_string())
            .or_insert(DeviceInfo {
                tools: vec![],
                last_seen: Instant::now(),
                online: true,
            });
        info.tools = tools.to_vec();
        info.last_seen = Instant::now();
        info.online = true;
    }

    pub fn mark_offline(&mut self, device_id: &str) {
        if let Some(info) = self.devices.get_mut(device_id) {
            info.online = false;
        }
    }

    pub fn is_online(&self, device_id: &str) -> bool {
        self.devices
            .get(device_id)
            .is_some_and(|info| info.online && info.last_seen.elapsed() < self.timeout)
    }

    pub fn online_devices(&self) -> Vec<&str> {
        self.devices
            .iter()
            .filter(|(_, info)| info.online && info.last_seen.elapsed() < self.timeout)
            .map(|(id, _)| id.as_str())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn register_device() {
        let mut mgr = DeviceManager::new(Duration::from_secs(120));
        mgr.update_heartbeat("dev-1", &["gpio_write".into()]);
        assert!(mgr.is_online("dev-1"));
        assert!(!mgr.is_online("dev-2"));
    }

    #[test]
    fn device_goes_offline_after_timeout() {
        let mut mgr = DeviceManager::new(Duration::from_secs(0));
        mgr.update_heartbeat("dev-1", &["gpio_write".into()]);
        // 超时为 0 秒，立即过期
        std::thread::sleep(Duration::from_millis(10));
        assert!(!mgr.is_online("dev-1"));
    }

    #[test]
    fn mark_offline_explicitly() {
        let mut mgr = DeviceManager::new(Duration::from_secs(120));
        mgr.update_heartbeat("dev-1", &["gpio_write".into()]);
        mgr.mark_offline("dev-1");
        assert!(!mgr.is_online("dev-1"));
    }

    #[test]
    fn list_online_devices() {
        let mut mgr = DeviceManager::new(Duration::from_secs(120));
        mgr.update_heartbeat("dev-1", &["gpio_write".into()]);
        mgr.update_heartbeat("dev-2", &["gpio_read".into()]);
        let online = mgr.online_devices();
        assert_eq!(online.len(), 2);
    }
}
