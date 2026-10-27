// Copyright (c) 2026 Ma Xin <maxin.zhang@users.noreply.github.com>
// SPDX-License-Identifier: MIT
//
// Reminder task model.
//
// Field naming and on-disk layout mirror the observable behaviour of the
// reference binary (12 schedule types, JSON keys documented in
// docs/reverse-analysis.md §3.5). The Rust struct is a clean-room
// reimplementation: it introduces a real enum between the variant-specific
// fields that the original shared inside a flat struct, so invalid states
// cannot be represented.

use serde::{Deserialize, Serialize};

use crate::time::{Date, DateTime, TimeOfDay};

/// Bit mask with bit0 = Monday ... bit6 = Sunday.
pub const WORKDAY_MASK: u8 = 0b001_1111;

/// Which monthly anchor a `Monthly` schedule uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MonthlyMode {
    /// A specific day number, clamped to the length of the month.
    Day,
    /// The last calendar day of the month.
    LastDay,
    /// The last Monday-Friday day of the month.
    LastWorkday,
}

/// The 12 schedule kinds, serialised with the same tokens as the original.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ScheduleType {
    Interval,
    Daily,
    Once,
    Weekly,
    Workdays,
    Monthly,
    MultiDaily,
    TimedInterval,
    Countdown,
    Pomodoro,
    AfterStartup,
    UsageBreak,
}

impl ScheduleType {
    /// Parse the on-disk token, accepting the exact set written by the
    /// reference implementation.
    pub fn from_token(token: &str) -> Option<Self> {
        Some(match token {
            "interval" => Self::Interval,
            "daily" => Self::Daily,
            "once" => Self::Once,
            "weekly" => Self::Weekly,
            "workdays" => Self::Workdays,
            "monthly" => Self::Monthly,
            "multiDaily" => Self::MultiDaily,
            "timedInterval" => Self::TimedInterval,
            "countdown" => Self::Countdown,
            "pomodoro" => Self::Pomodoro,
            "afterStartup" => Self::AfterStartup,
            "usageBreak" => Self::UsageBreak,
            _ => return None,
        })
    }

    pub fn as_token(self) -> &'static str {
        match self {
            Self::Interval => "interval",
            Self::Daily => "daily",
            Self::Once => "once",
            Self::Weekly => "weekly",
            Self::Workdays => "workdays",
            Self::Monthly => "monthly",
            Self::MultiDaily => "multiDaily",
            Self::TimedInterval => "timedInterval",
            Self::Countdown => "countdown",
            Self::Pomodoro => "pomodoro",
            Self::AfterStartup => "afterStartup",
            Self::UsageBreak => "usageBreak",
        }
    }

    /// Human label shown in the schedule picker.
    pub fn label(self) -> &'static str {
        match self {
            Self::Interval => "按间隔循环",
            Self::Daily => "每天固定时间",
            Self::Once => "指定日期时间（一次）",
            Self::Weekly => "每周指定日期",
            Self::Workdays => "仅工作日",
            Self::Monthly => "每月指定日期",
            Self::MultiDaily => "每天多个时间点",
            Self::TimedInterval => "限定时段循环",
            Self::Countdown => "倒计时",
            Self::Pomodoro => "番茄钟",
            Self::AfterStartup => "开机后提醒",
            Self::UsageBreak => "连续使用电脑提醒",
        }
    }

    pub const ALL: [Self; 12] = [
        Self::Interval,
        Self::Daily,
        Self::Once,
        Self::Weekly,
        Self::Workdays,
        Self::Monthly,
        Self::MultiDaily,
        Self::TimedInterval,
        Self::Countdown,
        Self::Pomodoro,
        Self::AfterStartup,
        Self::UsageBreak,
    ];
}

/// Variant-specific scheduling parameters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "scheduleType", rename_all = "camelCase")]
pub enum Schedule {
    /// Repeat every N minutes from the moment the schedule was armed.
    #[serde(rename_all = "camelCase")]
    Interval {
        /// Accepts both spellings so files written before the naming fix still load.
        #[serde(alias = "interval_minutes")]
        interval_minutes: i64,
    },
    /// Fixed wall-clock time, once per day.
    #[serde(rename_all = "camelCase")]
    Daily { #[serde(alias = "daily_time")] daily_time: TimeOfDay },
    /// One single absolute datetime.
    #[serde(rename_all = "camelCase")]
    Once { #[serde(alias = "once_at")] once_at: DateTime },
    /// Fixed wall-clock time on the selected weekdays.
    #[serde(rename_all = "camelCase")]
    Weekly { #[serde(alias = "weekdays_mask")] weekdays_mask: u8, #[serde(alias = "daily_time")] daily_time: TimeOfDay },
    /// Fixed wall-clock time, Monday to Friday.
    #[serde(rename_all = "camelCase")]
    Workdays { #[serde(alias = "daily_time")] daily_time: TimeOfDay },
    /// Fixed wall-clock time on a monthly anchor.
    #[serde(rename_all = "camelCase")]
    Monthly {
        #[serde(alias = "monthly_mode")] monthly_mode: MonthlyMode,
        #[serde(alias = "monthly_day")] monthly_day: u32,
        #[serde(alias = "daily_time")] daily_time: TimeOfDay,
    },
    /// Several fixed wall-clock times every day.
    #[serde(rename_all = "camelCase")]
    MultiDaily { #[serde(alias = "daily_times")] daily_times: Vec<TimeOfDay> },
    /// Repeat every N minutes but only inside a weekday + time window.
    #[serde(rename_all = "camelCase")]
    TimedInterval {
        #[serde(alias = "weekdays_mask")] weekdays_mask: u8,
        #[serde(alias = "active_start")] active_start: TimeOfDay,
        #[serde(alias = "active_end")] active_end: TimeOfDay,
        #[serde(alias = "interval_minutes")] interval_minutes: i64,
    },
    /// Fire once after N minutes.
    #[serde(rename_all = "camelCase")]
    Countdown { #[serde(alias = "countdown_minutes")] countdown_minutes: i64 },
    /// Alternating work and break intervals, repeated for a number of cycles.
    #[serde(rename_all = "camelCase")]
    Pomodoro {
        #[serde(alias = "work_minutes")] work_minutes: i64,
        #[serde(alias = "break_minutes")] break_minutes: i64,
        #[serde(alias = "cycles")] cycles: u32,
    },
    /// Fire N minutes after the application started.
    #[serde(rename_all = "camelCase")]
    AfterStartup { #[serde(alias = "startup_delay_minutes")] startup_delay_minutes: i64 },
    /// Fire after N minutes of continuous computer usage.
    #[serde(rename_all = "camelCase")]
    UsageBreak { #[serde(alias = "usage_minutes")] usage_minutes: i64 },
}

impl Schedule {
    pub fn schedule_type(&self) -> ScheduleType {
        match self {
            Self::Interval { .. } => ScheduleType::Interval,
            Self::Daily { .. } => ScheduleType::Daily,
            Self::Once { .. } => ScheduleType::Once,
            Self::Weekly { .. } => ScheduleType::Weekly,
            Self::Workdays { .. } => ScheduleType::Workdays,
            Self::Monthly { .. } => ScheduleType::Monthly,
            Self::MultiDaily { .. } => ScheduleType::MultiDaily,
            Self::TimedInterval { .. } => ScheduleType::TimedInterval,
            Self::Countdown { .. } => ScheduleType::Countdown,
            Self::Pomodoro { .. } => ScheduleType::Pomodoro,
            Self::AfterStartup { .. } => ScheduleType::AfterStartup,
            Self::UsageBreak { .. } => ScheduleType::UsageBreak,
        }
    }
}

/// Runtime state that must survive a restart.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PomodoroState {
    /// 1-based index of the phase in progress.
    pub current_cycle: u32,
    /// True while the break phase is running.
    pub in_break: bool,
}

/// A reminder task as stored on disk and manipulated at runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub message: String,
    #[serde(flatten)]
    pub schedule: Schedule,
    /// Absolute time of the next firing; `None` means "not armed yet".
    #[serde(default)]
    pub next_trigger: Option<DateTime>,
    /// Suppresses firing until this instant ("今日不再提醒" / "+10 分钟").
    #[serde(default)]
    pub muted_until: Option<DateTime>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub pomodoro: PomodoroState,
    /// Set when an `AfterStartup` task was armed for the current boot.
    #[serde(default)]
    pub startup_boot_marker: Option<String>,
}

fn default_true() -> bool {
    true
}

impl Task {
    pub fn new(id: impl Into<String>, title: impl Into<String>, schedule: Schedule) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            message: String::new(),
            schedule,
            next_trigger: None,
            muted_until: None,
            enabled: true,
            pomodoro: PomodoroState::default(),
            startup_boot_marker: None,
        }
    }

    pub fn schedule_type(&self) -> ScheduleType {
        self.schedule.schedule_type()
    }

    /// Whether the task can fire at `now`, ignoring schedule arithmetic.
    pub fn is_active_at(&self, now: &DateTime) -> bool {
        if !self.enabled {
            return false;
        }
        match &self.muted_until {
            Some(m) => now >= m,
            None => true,
        }
    }

    /// Render the schedule as the one-line Chinese description shown in lists.
    pub fn describe(&self) -> String {
        describe_schedule(&self.schedule)
    }
}

const WEEKDAY_NAMES: [&str; 7] = ["一", "二", "三", "四", "五", "六", "日"];

/// Join the masked weekday names, e.g. mask 0b101 → "一、三".
fn weekday_names(mask: u8) -> String {
    let names: Vec<&str> = (0..7)
        .filter(|i| mask & (1 << i) != 0)
        .map(|i| WEEKDAY_NAMES[i])
        .collect();
    if names.is_empty() {
        return String::new();
    }
    names.join("、")
}

fn describe_schedule(s: &Schedule) -> String {
    match s {
        Schedule::Interval { interval_minutes } => {
            let m = (*interval_minutes).max(1);
            if m % 60 == 0 {
                format!("每 {} 小时", m / 60)
            } else {
                format!("每 {} 分钟", m)
            }
        }
        Schedule::Daily { daily_time } => format!("每天 {}", daily_time.format_hm()),
        Schedule::Once { once_at } => format!("一次：{}", once_at.format_ymd_hm()),
        Schedule::Weekly {
            weekdays_mask,
            daily_time,
        } => format!("周{} {}", weekday_names(*weekdays_mask), daily_time.format_hm()),
        Schedule::Workdays { daily_time } => format!("工作日 {}", daily_time.format_hm()),
        Schedule::Monthly {
            monthly_mode,
            monthly_day,
            daily_time,
        } => {
            let anchor = match monthly_mode {
                MonthlyMode::Day => format!("{} 日", monthly_day),
                MonthlyMode::LastDay => "最后一天".to_string(),
                MonthlyMode::LastWorkday => "最后工作日".to_string(),
            };
            // `anchor` already ends with its unit ("1 日" / "最后一天"), so join with
            // a single space and no extra padding.
            format!("每月 {} {}", anchor, daily_time.format_hm())
        }
        Schedule::MultiDaily { daily_times } => {
            let joined: Vec<String> = daily_times.iter().map(|t| t.format_hm()).collect();
            format!("每天 {}", joined.join(", "))
        }
        Schedule::TimedInterval {
            weekdays_mask,
            active_start,
            active_end,
            interval_minutes,
        } => format!(
            "周{} {}-{} 每 {} 分钟",
            weekday_names(*weekdays_mask),
            active_start.format_hm(),
            active_end.format_hm(),
            (*interval_minutes).max(1)
        ),
        Schedule::Countdown { countdown_minutes } => {
            format!("倒计时 {} 分钟", (*countdown_minutes).max(1))
        }
        Schedule::Pomodoro {
            work_minutes,
            break_minutes,
            cycles,
        } => format!(
            "番茄钟 {}/{} 分钟 × {}",
            (*work_minutes).max(1),
            (*break_minutes).max(1),
            (*cycles).max(1)
        ),
        Schedule::AfterStartup {
            startup_delay_minutes,
        } => format!("开机后 {} 分钟", (*startup_delay_minutes).max(1)),
        Schedule::UsageBreak { usage_minutes } => {
            format!("连续使用 {} 分钟", (*usage_minutes).max(1))
        }
    }
}

/// Convenience constructor used by the "new task" flow.
pub fn default_schedule_for(kind: ScheduleType) -> Schedule {
    match kind {
        ScheduleType::Interval => Schedule::Interval {
            interval_minutes: 60,
        },
        ScheduleType::Daily => Schedule::Daily {
            daily_time: TimeOfDay::new(9, 0, 0),
        },
        // A placeholder only: `DateTime` has no "unset" value, so this is the
        // instant a brand-new one-shot task would use if the caller does not
        // supply one. The new-task form always replaces it with a time computed
        // from the wall clock, and `next_trigger` treats an expired one-shot as
        // "nothing to arm" rather than firing in the past.
        ScheduleType::Once => Schedule::Once {
            once_at: DateTime::new(Date::new(2099, 12, 31), TimeOfDay::new(9, 0, 0)),
        },
        ScheduleType::Weekly => Schedule::Weekly {
            weekdays_mask: 0b000_0001,
            daily_time: TimeOfDay::new(9, 0, 0),
        },
        ScheduleType::Workdays => Schedule::Workdays {
            daily_time: TimeOfDay::new(9, 0, 0),
        },
        ScheduleType::Monthly => Schedule::Monthly {
            monthly_mode: MonthlyMode::Day,
            monthly_day: 1,
            daily_time: TimeOfDay::new(9, 0, 0),
        },
        ScheduleType::MultiDaily => Schedule::MultiDaily {
            daily_times: vec![TimeOfDay::new(9, 0, 0)],
        },
        ScheduleType::TimedInterval => Schedule::TimedInterval {
            weekdays_mask: WORKDAY_MASK,
            active_start: TimeOfDay::new(9, 0, 0),
            active_end: TimeOfDay::new(18, 0, 0),
            interval_minutes: 60,
        },
        ScheduleType::Countdown => Schedule::Countdown {
            countdown_minutes: 25,
        },
        ScheduleType::Pomodoro => Schedule::Pomodoro {
            work_minutes: 25,
            break_minutes: 5,
            cycles: 4,
        },
        ScheduleType::AfterStartup => Schedule::AfterStartup {
            startup_delay_minutes: 5,
        },
        ScheduleType::UsageBreak => Schedule::UsageBreak { usage_minutes: 45 },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::Date;

    #[test]
    fn all_tokens_roundtrip() {
        for t in ScheduleType::ALL {
            assert_eq!(ScheduleType::from_token(t.as_token()), Some(t));
        }
        assert_eq!(ScheduleType::from_token("bogus"), None);
    }

    #[test]
    fn twelve_types_have_distinct_labels() {
        let labels: Vec<&str> = ScheduleType::ALL.iter().map(|t| t.label()).collect();
        let unique: std::collections::HashSet<&&str> = labels.iter().collect();
        assert_eq!(unique.len(), 12);
    }

    #[test]
    fn interval_description_switches_unit() {
        let hourly = Schedule::Interval {
            interval_minutes: 120,
        };
        let minutely = Schedule::Interval {
            interval_minutes: 25,
        };
        assert_eq!(describe_schedule(&hourly), "每 2 小时");
        assert_eq!(describe_schedule(&minutely), "每 25 分钟");
    }

    #[test]
    fn monthly_modes_use_own_anchor_wording() {
        let day = Schedule::Monthly {
            monthly_mode: MonthlyMode::Day,
            monthly_day: 15,
            daily_time: TimeOfDay::new(10, 30, 0),
        };
        assert_eq!(describe_schedule(&day), "每月 15 日 10:30");

        let last = Schedule::Monthly {
            monthly_mode: MonthlyMode::LastDay,
            monthly_day: 1,
            daily_time: TimeOfDay::new(10, 30, 0),
        };
        assert_eq!(describe_schedule(&last), "每月 最后一天 10:30");
    }

    #[test]
    fn weekly_description_lists_masked_days() {
        let s = Schedule::Weekly {
            weekdays_mask: 0b000_0101,
            daily_time: TimeOfDay::new(9, 0, 0),
        };
        assert_eq!(describe_schedule(&s), "周一、三 09:00");
    }

    #[test]
    fn muted_until_blocks_and_then_releases() {
        let mut task = Task::new("t1", "喝水", Schedule::Interval { interval_minutes: 60 });
        let now = DateTime::new(Date::new(2026, 10, 22), TimeOfDay::new(10, 0, 0));

        assert!(task.is_active_at(&now));

        task.muted_until = Some(DateTime::new(Date::new(2026, 10, 22), TimeOfDay::new(12, 0, 0)));
        assert!(!task.is_active_at(&now));

        let after = DateTime::new(Date::new(2026, 10, 22), TimeOfDay::new(12, 0, 0));
        assert!(task.is_active_at(&after));
    }

    #[test]
    fn disabled_task_is_never_active() {
        let mut task = Task::new("t1", "x", Schedule::Countdown { countdown_minutes: 5 });
        task.enabled = false;
        let now = DateTime::new(Date::new(2026, 10, 22), TimeOfDay::new(10, 0, 0));
        assert!(!task.is_active_at(&now));
    }
}
