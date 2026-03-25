use chrono::{NaiveTime, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 计划任务类型
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum CronType {
    /// 周期性任务（每 N 分钟执行）
    #[serde(rename = "periodic")]
    Periodic { interval_minutes: u32 },
    /// 每日定时任务
    #[serde(rename = "daily")]
    Daily { hour: u8, minute: u8 },
    /// 一次性延时任务
    #[serde(rename = "once")]
    Once { delay_minutes: u32 },
}

/// 一条计划任务
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronJob {
    pub id: u32,
    pub device_id: String,
    pub cron_type: CronType,
    pub action: String,
    pub created_at: i64,
    pub last_run: Option<i64>,
    pub enabled: bool,
}

/// 管理所有设备的计划任务
pub struct Scheduler {
    jobs: Vec<CronJob>,
    next_id: u32,
    /// device_id -> timezone
    timezones: HashMap<String, String>,
}

impl Scheduler {
    pub fn new() -> Self {
        Self {
            jobs: Vec::new(),
            next_id: 1,
            timezones: HashMap::new(),
        }
    }

    /// 设置设备时区（供 daily 任务使用）
    pub fn set_device_timezone(&mut self, device_id: &str, tz: &str) {
        self.timezones.insert(device_id.to_string(), tz.to_string());
    }

    /// 创建计划任务
    pub fn create_job(&mut self, device_id: &str, cron_type: CronType, action: &str) -> &CronJob {
        let job = CronJob {
            id: self.next_id,
            device_id: device_id.to_string(),
            cron_type,
            action: action.to_string(),
            created_at: Utc::now().timestamp(),
            last_run: None,
            enabled: true,
        };
        self.next_id += 1;
        self.jobs.push(job);
        self.jobs.last().unwrap()
    }

    /// 列出设备的所有计划任务
    pub fn list_jobs(&self, device_id: &str) -> Vec<&CronJob> {
        self.jobs
            .iter()
            .filter(|j| j.device_id == device_id && j.enabled)
            .collect()
    }

    /// 删除计划任务
    pub fn delete_job(&mut self, device_id: &str, job_id: u32) -> bool {
        if let Some(job) = self
            .jobs
            .iter_mut()
            .find(|j| j.id == job_id && j.device_id == device_id)
        {
            job.enabled = false;
            true
        } else {
            false
        }
    }

    /// 检查并返回所有需要执行的任务（由 tick 循环调用）
    pub fn check_due_jobs(&mut self) -> Vec<(String, String)> {
        let now = Utc::now().timestamp();
        let mut due: Vec<(String, String)> = Vec::new();

        for job in self.jobs.iter_mut().filter(|j| j.enabled) {
            let should_run = match &job.cron_type {
                CronType::Periodic { interval_minutes } => {
                    let interval_secs = *interval_minutes as i64 * 60;
                    let last = job.last_run.unwrap_or(job.created_at);
                    now - last >= interval_secs
                }
                CronType::Daily { hour, minute } => {
                    let tz_str = self
                        .timezones
                        .get(&job.device_id)
                        .map(|s| s.as_str())
                        .unwrap_or("UTC");
                    let tz: Tz = tz_str.parse().unwrap_or(chrono_tz::UTC);
                    let local_now = Utc::now().with_timezone(&tz);
                    let target = NaiveTime::from_hms_opt(*hour as u32, *minute as u32, 0)
                        .unwrap_or(NaiveTime::from_hms_opt(0, 0, 0).unwrap());

                    let is_time = local_now.time().hour() == target.hour()
                        && local_now.time().minute() == target.minute();

                    // 防止同一分钟内重复执行
                    let last_run_minute = job.last_run.map(|t| t / 60).unwrap_or(0);
                    let current_minute = now / 60;
                    is_time && last_run_minute != current_minute
                }
                CronType::Once { delay_minutes } => {
                    let trigger_at = job.created_at + (*delay_minutes as i64 * 60);
                    job.last_run.is_none() && now >= trigger_at
                }
            };

            if should_run {
                due.push((job.device_id.clone(), job.action.clone()));
                job.last_run = Some(now);

                // 一次性任务执行后自动禁用
                if matches!(job.cron_type, CronType::Once { .. }) {
                    job.enabled = false;
                }
            }
        }

        due
    }
}

use chrono::Timelike;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_and_list_jobs() {
        let mut sched = Scheduler::new();
        sched.create_job(
            "dev-01",
            CronType::Periodic {
                interval_minutes: 30,
            },
            "gpio_write living_room_light true",
        );
        sched.create_job(
            "dev-01",
            CronType::Daily { hour: 7, minute: 0 },
            "gpio_write bedroom_light true",
        );

        let jobs = sched.list_jobs("dev-01");
        assert_eq!(jobs.len(), 2);
        assert_eq!(jobs[0].id, 1);
        assert_eq!(jobs[1].id, 2);
    }

    #[test]
    fn delete_job() {
        let mut sched = Scheduler::new();
        sched.create_job(
            "dev-01",
            CronType::Periodic {
                interval_minutes: 10,
            },
            "some action",
        );
        assert!(sched.delete_job("dev-01", 1));
        assert!(sched.list_jobs("dev-01").is_empty());
    }

    #[test]
    fn delete_nonexistent_job() {
        let mut sched = Scheduler::new();
        assert!(!sched.delete_job("dev-01", 999));
    }

    #[test]
    fn periodic_job_triggers_after_interval() {
        let mut sched = Scheduler::new();
        sched.create_job(
            "dev-01",
            CronType::Periodic {
                interval_minutes: 0, // 立即触发
            },
            "test action",
        );
        let due = sched.check_due_jobs();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].1, "test action");
    }

    #[test]
    fn once_job_triggers_and_disables() {
        let mut sched = Scheduler::new();
        sched.create_job(
            "dev-01",
            CronType::Once { delay_minutes: 0 },
            "one-time action",
        );
        let due = sched.check_due_jobs();
        assert_eq!(due.len(), 1);

        // 第二次不应再触发
        let due = sched.check_due_jobs();
        assert!(due.is_empty());
    }

    #[test]
    fn device_isolation() {
        let mut sched = Scheduler::new();
        sched.create_job(
            "dev-01",
            CronType::Periodic {
                interval_minutes: 10,
            },
            "action A",
        );
        sched.create_job(
            "dev-02",
            CronType::Periodic {
                interval_minutes: 10,
            },
            "action B",
        );
        assert_eq!(sched.list_jobs("dev-01").len(), 1);
        assert_eq!(sched.list_jobs("dev-02").len(), 1);
    }
}
