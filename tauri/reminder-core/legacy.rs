// Copyright (c) 2026 Ma Xin <maxin.zhang@users.noreply.github.com>
// SPDX-License-Identifier: MIT
//
// Import of the reference implementation's config file.
//
// STATUS (2026-10): kept, but not wired into the Tauri front end yet. The
// module is public API of reminder-core and `docs/feature-parity.md` #24 plus
// the README list it as the v1 migration path, so it must not be deleted; the
// Tauri `main.rs` never gained the call (`legacy::import_from_disk` has no
// caller outside this file's tests). Wiring it in means: on startup, when the
// new store does not exist, try the legacy path first and persist the result.
// Until then treat this as an exported, tested library function awaiting a
// caller.
//
// The original program stored its tasks at
//   %APPDATA%\LocalTools\WorkReminder\reminders.json
// as a flat object per task: every schedule kind's parameters lived side by side
// in the same struct (see docs/reverse-analysis.md §3.5), times were "HH:mm:ss"
// strings, and datetimes were ISO-8601 strings produced by
// QJsonValue::fromVariant(QDateTime).
//
// This module reads that shape and produces the typed model, so an existing
// installation upgrades without retyping anything. The import is deliberately
// lossy in one direction only: unknown keys are ignored, but a task that cannot
// be understood is reported rather than silently dropped.

use serde::Deserialize;

use crate::model::{MonthlyMode, Schedule, Task, WORKDAY_MASK};
use crate::time::{Date, DateTime, TimeOfDay};

/// Legacy config directory name, as written by the original.
pub const LEGACY_ORG: &str = "LocalTools";
pub const LEGACY_APP: &str = "WorkReminder";

/// One task as the original program wrote it. All fields optional so a partial
/// or hand-edited file still yields something useful.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacyTask {
    #[serde(default)]
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    message: String,
    #[serde(default)]
    schedule_type: Option<String>,

    #[serde(default)]
    interval_minutes: Option<i64>,
    #[serde(default)]
    daily_time: Option<String>,
    #[serde(default)]
    weekdays_mask: Option<u8>,
    #[serde(default)]
    monthly_mode: Option<String>,
    #[serde(default)]
    monthly_day: Option<u32>,
    #[serde(default)]
    daily_times: Vec<String>,
    #[serde(default)]
    active_start: Option<String>,
    #[serde(default)]
    active_end: Option<String>,

    #[serde(default)]
    countdown_minutes: Option<i64>,
    #[serde(default)]
    pomodoro_work_minutes: Option<i64>,
    #[serde(default)]
    pomodoro_break_minutes: Option<i64>,
    #[serde(default)]
    pomodoro_cycles: Option<u32>,
    #[serde(default)]
    pomodoro_current_cycle: Option<u32>,
    #[serde(default)]
    pomodoro_in_break: Option<bool>,

    #[serde(default)]
    startup_delay_minutes: Option<i64>,
    #[serde(default)]
    usage_minutes: Option<i64>,

    #[serde(default)]
    once_at: Option<String>,
    #[serde(default)]
    next_trigger: Option<String>,
    #[serde(default)]
    muted_until: Option<String>,
    #[serde(default)]
    enabled: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
struct LegacyStore {
    #[serde(default)]
    version: Option<u32>,
    #[serde(default)]
    tasks: Vec<LegacyTask>,
}

/// Outcome of an import attempt.
pub struct ImportReport {
    pub tasks: Vec<Task>,
    /// Tasks that were present but could not be converted, with the reason.
    pub skipped: Vec<String>,
    /// Legacy schema version found in the file, for diagnostics.
    pub source_version: Option<u32>,
}

/// Parse the legacy JSON payload.
///
/// A malformed document is an error; individual broken tasks are collected in
/// `skipped` so one bad entry cannot block the whole upgrade.
pub fn import_json(raw: &str) -> Result<ImportReport, String> {
    // The legacy writer always emitted a JSON object. Reject anything else so a
    // wrong file (for example the new store's own array form) is not silently
    // imported as empty.
    if !raw.trim_start().starts_with('{') {
        return Err("配置文件格式不正确：顶层应为 JSON 对象".to_string());
    }
    let store: LegacyStore =
        serde_json::from_str(raw).map_err(|e| format!("配置文件格式不正确：{}", e))?;

    let mut tasks = Vec::new();
    let mut skipped = Vec::new();
    for (index, legacy) in store.tasks.into_iter().enumerate() {
        match convert(legacy, index) {
            Ok(task) => tasks.push(task),
            Err(reason) => skipped.push(format!("第 {} 条：{}", index + 1, reason)),
        }
    }
    Ok(ImportReport {
        tasks,
        skipped,
        source_version: store.version,
    })
}

fn convert(legacy: LegacyTask, index: usize) -> Result<Task, String> {
    let kind = legacy
        .schedule_type
        .as_deref()
        .unwrap_or("interval")
        .to_string();

    let schedule = match kind.as_str() {
        "interval" => Schedule::Interval {
            interval_minutes: positive(legacy.interval_minutes, 60),
        },
        "daily" => Schedule::Daily {
            daily_time: time_or(legacy.daily_time.as_deref(), (9, 0))?,
        },
        "once" => {
            let at = legacy
                .once_at
                .as_deref()
                .ok_or("一次性提醒缺少 onceAt")?;
            Schedule::Once {
                once_at: parse_datetime(at).ok_or_else(|| format!("无法识别的时间：{}", at))?,
            }
        }
        "weekly" => Schedule::Weekly {
            weekdays_mask: legacy.weekdays_mask.unwrap_or(0b000_0001),
            daily_time: time_or(legacy.daily_time.as_deref(), (9, 0))?,
        },
        "workdays" => Schedule::Workdays {
            daily_time: time_or(legacy.daily_time.as_deref(), (9, 0))?,
        },
        "monthly" => Schedule::Monthly {
            monthly_mode: monthly_mode(legacy.monthly_mode.as_deref(), legacy.monthly_day),
            monthly_day: legacy.monthly_day.unwrap_or(1).clamp(1, 31),
            daily_time: time_or(legacy.daily_time.as_deref(), (9, 0))?,
        },
        "multiDaily" => {
            let mut times = Vec::new();
            for text in &legacy.daily_times {
                if let Some(t) = TimeOfDay::parse(text) {
                    times.push(t);
                }
            }
            if times.is_empty() {
                times.push(TimeOfDay::new(9, 0, 0));
            }
            Schedule::MultiDaily { daily_times: times }
        }
        "timedInterval" => Schedule::TimedInterval {
            weekdays_mask: legacy.weekdays_mask.unwrap_or(WORKDAY_MASK),
            active_start: time_or(legacy.active_start.as_deref(), (9, 0))?,
            active_end: time_or(legacy.active_end.as_deref(), (18, 0))?,
            interval_minutes: positive(legacy.interval_minutes, 60),
        },
        "countdown" => Schedule::Countdown {
            countdown_minutes: positive(legacy.countdown_minutes, 25),
        },
        "pomodoro" => Schedule::Pomodoro {
            work_minutes: positive(legacy.pomodoro_work_minutes, 25),
            break_minutes: positive(legacy.pomodoro_break_minutes, 5),
            cycles: legacy.pomodoro_cycles.unwrap_or(4).clamp(1, 100),
        },
        "afterStartup" => Schedule::AfterStartup {
            startup_delay_minutes: positive(legacy.startup_delay_minutes, 5),
        },
        "usageBreak" => Schedule::UsageBreak {
            usage_minutes: positive(legacy.usage_minutes, 45),
        },
        other => return Err(format!("未知的提醒方式：{}", other)),
    };

    let mut task = Task::new(
        if legacy.id.trim().is_empty() {
            format!("legacy-{}", index)
        } else {
            legacy.id
        },
        if legacy.title.trim().is_empty() {
            "（未命名提醒）".to_string()
        } else {
            legacy.title
        },
        schedule,
    );
    task.message = legacy.message;
    task.enabled = legacy.enabled.unwrap_or(true);

    // Carry the runtime anchors over so an upgrade does not re-fire a reminder
    // the user already dismissed today.
    task.next_trigger = legacy.next_trigger.as_deref().and_then(parse_datetime);
    task.muted_until = legacy.muted_until.as_deref().and_then(parse_datetime);

    if let Schedule::Pomodoro { .. } = task.schedule {
        task.pomodoro.current_cycle = legacy.pomodoro_current_cycle.unwrap_or(0);
        task.pomodoro.in_break = legacy.pomodoro_in_break.unwrap_or(false);
    }

    Ok(task)
}

fn positive(value: Option<i64>, fallback: i64) -> i64 {
    match value {
        Some(v) if v > 0 => v,
        _ => fallback,
    }
}

fn time_or(text: Option<&str>, fallback: (u32, u32)) -> Result<TimeOfDay, String> {
    match text {
        Some(t) => TimeOfDay::parse(t).ok_or_else(|| format!("无法识别的时间：{}", t)),
        None => Ok(TimeOfDay::new(fallback.0, fallback.1, 0)),
    }
}

/// The original wrote `monthlyMode` as a token: "lastDay", "lastWorkday", "day"
/// (see the string table at 0xbc2da0 / 0xbc2dc8 / 0xbc4bf8). It also accepted a
/// numeric form in early builds, so both are handled.
fn monthly_mode(token: Option<&str>, day: Option<u32>) -> MonthlyMode {
    match token.map(|t| t.trim()) {
        Some("lastWorkday") | Some("1") => MonthlyMode::LastWorkday,
        Some("lastDay") | Some("2") => MonthlyMode::LastDay,
        Some("day") | Some("0") => MonthlyMode::Day,
        // No token: fall back on the day value the way the original did.
        None => {
            if day.unwrap_or(0) == 31 {
                MonthlyMode::LastDay
            } else {
                MonthlyMode::Day
            }
        }
        _ => MonthlyMode::Day,
    }
}

/// Parse the datetime forms the original could emit.
///
/// Primary form is Qt's `QDateTime::toString(Qt::ISODate)`, e.g.
/// `2026-10-22T17:30:00`. Bare dates and space-separated forms are accepted too
/// because hand-edited files are common.
pub fn parse_datetime(text: &str) -> Option<DateTime> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    // 2026-10-22T17:30:00 or 2026-10-22T17:30 or 2026-10-22 17:30[:00]
    let (date_part, time_part) = t
        .split_once('T')
        .or_else(|| t.split_once(' '))
        .unwrap_or((t, "00:00:00"));

    let mut dparts = date_part.split('-');
    let year: i32 = dparts.next()?.parse().ok()?;
    let month: u32 = dparts.next()?.parse().ok()?;
    let day: u32 = dparts.next()?.parse().ok()?;
    if !(1..=12).contains(&month) || day == 0 || day > Date::days_in_month(year, month) {
        return None;
    }
    // Trim a trailing timezone designator, which the original never emitted but
    // other Qt builds can add.
    let time_clean = time_part
        .trim_end_matches('Z')
        .split(['+', '.'])
        .next()
        .unwrap_or(time_part);
    let time = TimeOfDay::parse(time_clean)?;
    Some(DateTime::new(Date::new(year, month, day), time))
}

/// Path of the legacy config file under the current user profile.
pub fn legacy_path() -> Option<std::path::PathBuf> {
    std::env::var_os("APPDATA").map(|base| {
        let mut p = std::path::PathBuf::from(base);
        p.push(LEGACY_ORG);
        p.push(LEGACY_APP);
        p.push("reminders.json");
        p
    })
}

/// Read and convert the legacy file.
///
/// `Ok(None)` means there is nothing to import (no legacy file present).
pub fn import_from_disk(path: &std::path::Path) -> Result<Option<ImportReport>, String> {
    match std::fs::read_to_string(path) {
        Ok(raw) => {
            if raw.trim().is_empty() {
                return Ok(None);
            }
            import_json(&raw).map(Some)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("无法读取旧版配置：{}", e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_every_legacy_schedule_kind() {
        let raw = r#"{
          "version": 1,
          "tasks": [
            {"id":"a","title":"间隔","scheduleType":"interval","intervalMinutes":30},
            {"id":"b","title":"每天","scheduleType":"daily","dailyTime":"09:00:00"},
            {"id":"c","title":"一次","scheduleType":"once","onceAt":"2026-12-31T17:30:00"},
            {"id":"d","title":"每周","scheduleType":"weekly","weekdaysMask":5,"dailyTime":"09:00:00"},
            {"id":"e","title":"工作日","scheduleType":"workdays","dailyTime":"09:30:00"},
            {"id":"f","title":"每月","scheduleType":"monthly","monthlyMode":"lastWorkday","monthlyDay":31,"dailyTime":"10:00:00"},
            {"id":"g","title":"多时点","scheduleType":"multiDaily","dailyTimes":["09:00:00","17:30:00"]},
            {"id":"h","title":"时段","scheduleType":"timedInterval","weekdaysMask":31,"activeStart":"09:00:00","activeEnd":"18:00:00","intervalMinutes":45},
            {"id":"i","title":"倒计时","scheduleType":"countdown","countdownMinutes":25},
            {"id":"j","title":"番茄","scheduleType":"pomodoro","pomodoroWorkMinutes":25,"pomodoroBreakMinutes":5,"pomodoroCycles":4},
            {"id":"k","title":"开机","scheduleType":"afterStartup","startupDelayMinutes":5},
            {"id":"l","title":"久坐","scheduleType":"usageBreak","usageMinutes":45}
          ]
        }"#;
        let report = import_json(raw).unwrap();
        assert_eq!(report.tasks.len(), 12, "skipped: {:?}", report.skipped);
        assert!(report.skipped.is_empty());
        assert_eq!(report.source_version, Some(1));

        let kinds: Vec<&str> = report
            .tasks
            .iter()
            .map(|t| t.schedule_type().as_token())
            .collect();
        assert_eq!(
            kinds,
            vec![
                "interval",
                "daily",
                "once",
                "weekly",
                "workdays",
                "monthly",
                "multiDaily",
                "timedInterval",
                "countdown",
                "pomodoro",
                "afterStartup",
                "usageBreak"
            ]
        );
    }

    #[test]
    fn converts_field_values() {
        let raw = r#"{"tasks":[
            {"id":"w","title":"周会","scheduleType":"weekly","weekdaysMask":5,"dailyTime":"09:15:00"},
            {"id":"m","title":"月结","scheduleType":"monthly","monthlyMode":"lastWorkday","monthlyDay":31,"dailyTime":"10:00:00"},
            {"id":"t","title":"时段","scheduleType":"timedInterval","weekdaysMask":31,"activeStart":"09:00:00","activeEnd":"18:00:00","intervalMinutes":45},
            {"id":"p","title":"番茄","scheduleType":"pomodoro","pomodoroWorkMinutes":50,"pomodoroBreakMinutes":10,"pomodoroCycles":3,"pomodoroCurrentCycle":1,"pomodoroInBreak":true}
        ]}"#;
        let report = import_json(raw).unwrap();
        assert_eq!(report.tasks.len(), 4);

        assert!(matches!(
            report.tasks[0].schedule,
            Schedule::Weekly {
                weekdays_mask: 5,
                daily_time: TimeOfDay { hour: 9, minute: 15, .. }
            }
        ));
        assert!(matches!(
            report.tasks[1].schedule,
            Schedule::Monthly {
                monthly_mode: MonthlyMode::LastWorkday,
                ..
            }
        ));
        assert!(matches!(
            report.tasks[2].schedule,
            Schedule::TimedInterval {
                interval_minutes: 45,
                ..
            }
        ));
        assert!(matches!(
            report.tasks[3].schedule,
            Schedule::Pomodoro {
                work_minutes: 50,
                break_minutes: 10,
                cycles: 3
            }
        ));
        assert_eq!(report.tasks[3].pomodoro.current_cycle, 1);
        assert!(report.tasks[3].pomodoro.in_break);
    }

    #[test]
    fn carries_runtime_anchors_so_upgrades_do_not_re_fire() {
        let raw = r#"{"tasks":[
            {"id":"a","title":"x","scheduleType":"daily","dailyTime":"09:00:00",
             "nextTrigger":"2026-10-23T09:00:00","mutedUntil":"2026-10-22T12:00:00","enabled":false}
        ]}"#;
        let report = import_json(raw).unwrap();
        let task = &report.tasks[0];
        assert_eq!(
            task.next_trigger,
            Some(DateTime::new(
                Date::new(2026, 10, 23),
                TimeOfDay::new(9, 0, 0)
            ))
        );
        assert_eq!(
            task.muted_until,
            Some(DateTime::new(
                Date::new(2026, 10, 22),
                TimeOfDay::new(12, 0, 0)
            ))
        );
        assert!(!task.enabled);
    }

    #[test]
    fn unknown_schedule_kind_is_reported_not_dropped_silently() {
        let raw = r#"{"tasks":[
            {"id":"a","title":"x","scheduleType":"telepathy"},
            {"id":"b","title":"ok","scheduleType":"interval","intervalMinutes":10}
        ]}"#;
        let report = import_json(raw).unwrap();
        assert_eq!(report.tasks.len(), 1);
        assert_eq!(report.skipped.len(), 1);
        assert!(report.skipped[0].contains("telepathy"));
        assert!(report.skipped[0].contains("第 1 条"));
    }

    #[test]
    fn missing_title_and_id_get_placeholders() {
        let raw = r#"{"tasks":[{"scheduleType":"interval","intervalMinutes":5}]}"#;
        let report = import_json(raw).unwrap();
        let task = &report.tasks[0];
        assert_eq!(task.title, "（未命名提醒）");
        assert!(task.id.starts_with("legacy-"));
    }

    #[test]
    fn non_positive_durations_fall_back_to_defaults() {
        let raw = r#"{"tasks":[
            {"id":"a","title":"x","scheduleType":"interval","intervalMinutes":0},
            {"id":"b","title":"y","scheduleType":"countdown","countdownMinutes":-5}
        ]}"#;
        let report = import_json(raw).unwrap();
        assert!(matches!(
            report.tasks[0].schedule,
            Schedule::Interval {
                interval_minutes: 60
            }
        ));
        assert!(matches!(
            report.tasks[1].schedule,
            Schedule::Countdown {
                countdown_minutes: 25
            }
        ));
    }

    #[test]
    fn datetime_parser_accepts_qt_iso_and_loose_forms() {
        let expected = DateTime::new(Date::new(2026, 10, 22), TimeOfDay::new(17, 30, 0));
        assert_eq!(parse_datetime("2026-10-22T17:30:00"), Some(expected));
        assert_eq!(parse_datetime("2026-10-22T17:30"), Some(expected));
        assert_eq!(parse_datetime("2026-10-22 17:30:00"), Some(expected));
        assert_eq!(parse_datetime("2026-10-22 17:30"), Some(expected));
        assert_eq!(parse_datetime("2026-10-22"), Some(DateTime::new(
            Date::new(2026, 10, 22),
            TimeOfDay::new(0, 0, 0)
        )));
        // Timezone designators are tolerated and dropped.
        assert_eq!(parse_datetime("2026-10-22T17:30:00Z"), Some(expected));
        assert_eq!(parse_datetime("2026-10-22T17:30:00.500"), Some(expected));
        // Invalid inputs.
        assert_eq!(parse_datetime(""), None);
        assert_eq!(parse_datetime("not a date"), None);
        assert_eq!(parse_datetime("2026-02-30T10:00:00"), None);
        assert_eq!(parse_datetime("2026-13-01T10:00:00"), None);
    }

    #[test]
    fn monthly_mode_accepts_tokens_and_numeric_aliases() {
        assert_eq!(monthly_mode(Some("lastDay"), None), MonthlyMode::LastDay);
        assert_eq!(
            monthly_mode(Some("lastWorkday"), None),
            MonthlyMode::LastWorkday
        );
        assert_eq!(monthly_mode(Some("day"), None), MonthlyMode::Day);
        assert_eq!(monthly_mode(Some("2"), None), MonthlyMode::LastDay);
        assert_eq!(monthly_mode(Some("1"), None), MonthlyMode::LastWorkday);
        assert_eq!(monthly_mode(Some("0"), None), MonthlyMode::Day);
        // Untokened files: day 31 was how "last day" was expressed.
        assert_eq!(monthly_mode(None, Some(31)), MonthlyMode::LastDay);
        assert_eq!(monthly_mode(None, Some(15)), MonthlyMode::Day);
    }

    #[test]
    fn malformed_document_is_an_error() {
        assert!(import_json("{ not json").is_err());
        assert!(import_json("[]").is_err());
    }

    #[test]
    fn missing_legacy_file_reports_nothing_to_do() {
        let dir = std::env::temp_dir().join("work-reminder-test-legacy");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        crate::store::register_temp_cleanup(dir.clone());
        let path = dir.join("reminders.json");
        assert!(import_from_disk(&path).unwrap().is_none());

        std::fs::write(&path, b"{\"tasks\":[]}").unwrap();
        let report = import_from_disk(&path).unwrap().unwrap();
        assert!(report.tasks.is_empty());
    }

    #[test]
    fn imported_tasks_roundtrip_through_the_new_store() {
        let raw = r#"{"tasks":[
            {"id":"a","title":"喝水","message":"接杯水","scheduleType":"interval","intervalMinutes":30,
             "nextTrigger":"2026-10-22T16:00:00","enabled":true}
        ]}"#;
        let report = import_json(raw).unwrap();
        let dir = std::env::temp_dir().join("work-reminder-test-import");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        crate::store::register_temp_cleanup(dir.clone());
        let path = dir.join("reminders.json");

        let mut store = crate::store::Store::default();
        store.tasks = report.tasks;
        crate::store::save(&path, &store).unwrap();
        let loaded = crate::store::load(&path).unwrap();
        assert_eq!(loaded.tasks.len(), 1);
        assert_eq!(loaded.tasks[0].title, "喝水");
        assert_eq!(loaded.tasks[0].message, "接杯水");
    }
}
