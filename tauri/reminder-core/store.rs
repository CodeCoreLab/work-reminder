// Copyright (c) 2026 Ma Xin <maxin.zhang@users.noreply.github.com>
// SPDX-License-Identifier: MIT
//
// Persistence: one JSON document, atomically replaced.
//
// Mirrors the observable behaviour at docs/reverse-analysis.md §3.6: the
// reference implementation read/wrote a single `reminders.json` under the
// per-user config directory and committed it through a temporary file
// (`QSaveFile`). Same atomic-write contract, without the Qt dependency.

use std::fs;
use std::io;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::model::Task;

/// Bumped whenever the on-disk shape changes; `load` refuses unknown futures.
pub const SCHEMA_VERSION: u32 = 1;

/// Root object of `reminders.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Store {
    pub version: u32,
    #[serde(default)]
    pub tasks: Vec<Task>,
}

impl Default for Store {
    fn default() -> Self {
        Self {
            version: SCHEMA_VERSION,
            tasks: Vec::new(),
        }
    }
}

/// Errors surfaced to the UI as `读取配置失败` / `保存失败` equivalents.
#[derive(Debug)]
pub enum StoreError {
    Io(io::Error),
    Parse(String),
    UnsupportedVersion(u32),
    Serialize(String),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "无法读取配置文件：{}", e),
            Self::Parse(e) => write!(f, "配置文件格式不正确：{}", e),
            Self::UnsupportedVersion(v) => {
                write!(f, "配置文件版本 {} 高于本程序支持的版本", v)
            }
            Self::Serialize(e) => write!(f, "无法保存配置：{}", e),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<io::Error> for StoreError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// Resolve the config file path.
///
/// `%APPDATA%\<org>\<app>\reminders.json`, matching the original
/// `QStandardPaths::AppConfigLocation` resolution
/// (`%APPDATA%\LocalTools\WorkReminder\reminders.json`).
pub fn config_path(org: &str, app: &str) -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join(org).join(app).join("reminders.json")
}

/// Read the store. A missing file is treated as an empty store; a malformed
/// file is an error, never a silent truncation.
///
/// When the file exists but cannot be parsed, a byte-for-byte copy is left next
/// to it (`reminders.json.<timestamp>.bad`) before the error is returned. The
/// caller proceeds with an empty task list, and the first save then overwrites
/// the unreadable file — without the copy, that overwrite destroys the only
/// evidence of what went wrong.
pub fn load(path: &Path) -> Result<Store, StoreError> {
    let raw = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Store::default()),
        Err(e) => return Err(StoreError::Io(e)),
    };
    if raw.trim().is_empty() {
        return Ok(Store::default());
    }
    let store: Store = serde_json::from_str(&raw).map_err(|e| {
        backup_corrupt_file(path);
        StoreError::Parse(e.to_string())
    })?;
    if store.version > SCHEMA_VERSION {
        return Err(StoreError::UnsupportedVersion(store.version));
    }
    Ok(store)
}

/// Copy an unparsable config file aside, best effort, so the next save cannot
/// destroy it. The name embeds the current local time so successive failures
/// never clobber each other.
fn backup_corrupt_file(path: &Path) {
    let stamp = crate::time::now_local();
    let backup = path.with_extension(format!(
        "json.{}{:02}{:02}-{:02}{:02}{:02}.bad",
        stamp.date.year,
        stamp.date.month,
        stamp.date.day,
        stamp.time.hour,
        stamp.time.minute,
        stamp.time.second
    ));
    if let Err(e) = fs::copy(path, &backup) {
        // Nothing better to do: the caller already reports the parse error, and
        // a failed backup must not mask it with a second one.
        eprintln!("store: 备份损坏配置失败 ({}): {e}", backup.display());
    }
}

/// Write the store atomically: temporary sibling file, fsync, then rename.
///
/// The temporary name carries the process id. A single fixed name means two
/// processes writing at once share one scratch file: one truncates while the other
/// is mid-write, and the loser's `rename` either fails or moves a torn payload into
/// place. With a per-process name each writer owns its own scratch file and the
/// rename stays the only cross-process critical section.
pub fn save(path: &Path, store: &Store) -> Result<(), StoreError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(store).map_err(|e| StoreError::Serialize(e.to_string()))?;

    // Unique per call: process id defeats cross-process collisions, the atomic
    // counter defeats in-process ones. Both are needed — a shared scratch file lets
    // one writer truncate another's payload and move a torn file into place.
    let seq = SAVE_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = path.with_extension(format!("json.{}.{}.tmp", std::process::id(), seq));
    // A failed write must not leave the scratch file behind to confuse the next run.
    let write_result = (|| -> std::io::Result<()> {
        // Write and flush through one handle. Opening a second read-only handle just to
        // fsync fails on Windows with "access denied" (os error 5) and would turn every
        // save into an error.
        let mut f = fs::File::create(&tmp)?;
        f.write_all(json.as_bytes())?;
        // Flush so a power loss does not leave a half-written file.
        f.sync_all()?;
        Ok(())
    })();
    if let Err(e) = write_result {
        let _ = fs::remove_file(&tmp);
        return Err(StoreError::Io(e));
    }
    // `rename` onto an existing file is atomic, but on Windows it fails with
    // "access denied" while another handle to the destination is still open — which is
    // exactly what happens when two instances save at the same moment, or when a
    // reader (the antivirus scanner, an editor, another instance loading the file) has
    // it open. A short bounded retry turns that transient collision into a success
    // instead of a user-visible save error.
    let mut last_err = None;
    for attempt in 0..RENAME_RETRIES {
        match fs::rename(&tmp, path) {
            Ok(()) => {
                last_err = None;
                break;
            }
            Err(e) => {
                last_err = Some(e);
                if attempt + 1 < RENAME_RETRIES {
                    std::thread::sleep(std::time::Duration::from_millis(
                        RENAME_BACKOFF_MS * (attempt as u64 + 1),
                    ));
                }
            }
        }
    }
    if let Some(e) = last_err {
        let _ = fs::remove_file(&tmp);
        return Err(StoreError::Io(e));
    }
    Ok(())
}

/// Distinguishes concurrent saves within one process.
static SAVE_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Attempts at the final rename before giving up.
const RENAME_RETRIES: u32 = 5;
/// Base backoff; grows linearly so the last wait is 5x this.
const RENAME_BACKOFF_MS: u64 = 20;

/// Scratch directories that should be deleted when the test binary exits.
#[cfg(test)]
static TEMP_DIRS: std::sync::Mutex<Vec<std::path::PathBuf>> = std::sync::Mutex::new(Vec::new());

/// Register a directory for deletion at process exit.
#[cfg(test)]
pub(crate) fn register_temp_cleanup(dir: std::path::PathBuf) {
    if let Ok(mut list) = TEMP_DIRS.lock() {
        if list.is_empty() {
            // Install the handler once, on first registration.
            extern "C" fn sweep() {
                if let Ok(list) = TEMP_DIRS.lock() {
                    for d in list.iter() {
                        let _ = std::fs::remove_dir_all(d);
                    }
                }
            }
            // SAFETY: `sweep` only touches a `Mutex` static and does no allocation
            // that could fault during exit.
            unsafe { libc_atexit(sweep) };
        }
        list.push(dir);
    }
}

/// `atexit` from the C runtime, called through the already-linked msvcrt.
///
/// Declared locally because the crate has no libc dependency and adding one would
/// outweigh a single symbol.
#[cfg(test)]
unsafe fn libc_atexit(f: extern "C" fn()) {
    extern "C" {
        fn atexit(cb: extern "C" fn()) -> std::os::raw::c_int;
    }
    atexit(f);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Schedule, Task};
    use crate::time::{Date, DateTime, TimeOfDay};

    /// Scratch directory for one test, removed when the test process exits.
    ///
    /// Fixed names (not PID-based) keep the number of temp entries constant across
    /// runs, and the registered cleanup means a test run leaves TEMP as it found it.
    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("work-reminder-test-{}", name));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        super::register_temp_cleanup(dir.clone());
        dir
    }

    #[test]
    fn on_disk_keys_are_camel_case() {
        // serde's `rename_all` on an enum does not reach the fields of its variants,
        // so each variant needs its own attribute. Without this the file used
        // snake_case while the documentation and the legacy importer used camelCase,
        // and a hand-edited file written per the docs failed to load.
        let task = Task::new(
            "1",
            "x",
            Schedule::TimedInterval {
                weekdays_mask: 31,
                active_start: TimeOfDay::new(9, 0, 0),
                active_end: TimeOfDay::new(18, 0, 0),
                interval_minutes: 45,
            },
        );
        let json = serde_json::to_string(&task).unwrap();
        for key in ["weekdaysMask", "activeStart", "activeEnd", "intervalMinutes"] {
            assert!(json.contains(key), "missing {key} in {json}");
        }
        for bad in ["weekdays_mask", "active_start", "interval_minutes"] {
            assert!(!json.contains(bad), "snake_case {bad} leaked into {json}");
        }
    }

    #[test]
    fn snake_case_keys_still_load() {
        // Files written before the naming fix must keep working.
        let dir = temp_dir("snake-alias");
        let path = dir.join("reminders.json");
        fs::write(
            &path,
            br#"{"version":1,"tasks":[
              {"id":"a","title":"old","scheduleType":"interval","interval_minutes":45,"enabled":true}
            ]}"#,
        )
        .unwrap();
        let store = load(&path).unwrap();
        assert_eq!(store.tasks.len(), 1);
        assert_eq!(
            store.tasks[0].schedule,
            Schedule::Interval {
                interval_minutes: 45
            }
        );
    }

    #[test]
    fn every_variant_roundtrips_in_both_spellings() {
        let dir = temp_dir("both-spellings");
        let path = dir.join("reminders.json");
        let mut store = Store::default();
        for (i, kind) in crate::model::ScheduleType::ALL.iter().enumerate() {
            store.tasks.push(Task::new(
                format!("t{i}"),
                kind.as_token(),
                crate::model::default_schedule_for(*kind),
            ));
        }
        save(&path, &store).unwrap();
        let first = load(&path).unwrap();
        // Rewrite using the snake_case spelling to prove the aliases accept it.
        let raw = fs::read_to_string(&path).unwrap();
        let snake = raw
            .replace("intervalMinutes", "interval_minutes")
            .replace("dailyTime", "daily_time")
            .replace("onceAt", "once_at")
            .replace("weekdaysMask", "weekdays_mask")
            .replace("monthlyMode", "monthly_mode")
            .replace("monthlyDay", "monthly_day")
            .replace("dailyTimes", "daily_times")
            .replace("activeStart", "active_start")
            .replace("activeEnd", "active_end")
            .replace("countdownMinutes", "countdown_minutes")
            .replace("workMinutes", "work_minutes")
            .replace("breakMinutes", "break_minutes")
            .replace("startupDelayMinutes", "startup_delay_minutes")
            .replace("usageMinutes", "usage_minutes");
        fs::write(&path, snake).unwrap();
        let second = load(&path).unwrap();
        assert_eq!(first.tasks.len(), second.tasks.len());
        for (a, b) in first.tasks.iter().zip(second.tasks.iter()) {
            assert_eq!(a.schedule, b.schedule, "schedule {} differs", a.id);
        }
    }

    #[test]
    fn missing_file_yields_empty_store() {
        let dir = temp_dir("missing");
        let path = dir.join("reminders.json");
        let store = load(&path).unwrap();
        assert_eq!(store.version, SCHEMA_VERSION);
        assert!(store.tasks.is_empty());
    }

    #[test]
    fn roundtrip_preserves_every_schedule_variant() {
        let dir = temp_dir("roundtrip");
        let path = dir.join("reminders.json");
        let mut store = Store::default();
        store.tasks = vec![
            Task::new("1", "a", Schedule::Interval { interval_minutes: 30 }),
            Task::new(
                "2",
                "b",
                Schedule::Daily {
                    daily_time: TimeOfDay::new(9, 0, 0),
                },
            ),
            Task::new(
                "3",
                "c",
                Schedule::Once {
                    once_at: DateTime::new(Date::new(2026, 12, 1), TimeOfDay::new(10, 0, 0)),
                },
            ),
            Task::new(
                "4",
                "d",
                Schedule::Weekly {
                    weekdays_mask: 0b001_0101,
                    daily_time: TimeOfDay::new(9, 0, 0),
                },
            ),
            Task::new(
                "5",
                "e",
                Schedule::Workdays {
                    daily_time: TimeOfDay::new(9, 0, 0),
                },
            ),
            Task::new(
                "6",
                "f",
                Schedule::Monthly {
                    monthly_mode: crate::model::MonthlyMode::LastWorkday,
                    monthly_day: 1,
                    daily_time: TimeOfDay::new(9, 0, 0),
                },
            ),
            Task::new(
                "7",
                "g",
                Schedule::MultiDaily {
                    daily_times: vec![TimeOfDay::new(9, 0, 0), TimeOfDay::new(17, 30, 0)],
                },
            ),
            Task::new(
                "8",
                "h",
                Schedule::TimedInterval {
                    weekdays_mask: 0b001_1111,
                    active_start: TimeOfDay::new(9, 0, 0),
                    active_end: TimeOfDay::new(18, 0, 0),
                    interval_minutes: 60,
                },
            ),
            Task::new("9", "i", Schedule::Countdown { countdown_minutes: 25 }),
            Task::new(
                "10",
                "j",
                Schedule::Pomodoro {
                    work_minutes: 25,
                    break_minutes: 5,
                    cycles: 4,
                },
            ),
            Task::new(
                "11",
                "k",
                Schedule::AfterStartup {
                    startup_delay_minutes: 5,
                },
            ),
            Task::new("12", "l", Schedule::UsageBreak { usage_minutes: 45 }),
        ];

        save(&path, &store).unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(loaded.tasks.len(), 12);
        for (a, b) in store.tasks.iter().zip(loaded.tasks.iter()) {
            assert_eq!(a.schedule, b.schedule, "schedule {} must roundtrip", a.id);
        }
    }

    #[test]
    fn malformed_json_is_an_error_not_a_reset() {
        let dir = temp_dir("malformed");
        let path = dir.join("reminders.json");
        fs::write(&path, b"{ not json ").unwrap();
        match load(&path) {
            Err(StoreError::Parse(_)) => {}
            other => panic!("expected parse error, got {:?}", other.map(|s| s.tasks.len())),
        }
    }

    #[test]
    fn malformed_load_leaves_a_backup_copy_behind() {
        // The next save overwrites the unreadable file, so the parse error path
        // must copy it aside first or the evidence is gone.
        let dir = temp_dir("backup");
        let path = dir.join("reminders.json");
        fs::write(&path, b"{ not json ").unwrap();
        assert!(load(&path).is_err());
        let backups: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.ends_with(".bad"))
            .collect();
        assert_eq!(backups.len(), 1, "one .bad backup expected: {backups:?}");
        let content = fs::read(dir.join(&backups[0])).unwrap();
        assert_eq!(content, b"{ not json ");
    }

    #[test]
    fn future_schema_version_is_rejected() {
        let dir = temp_dir("future");
        let path = dir.join("reminders.json");
        fs::write(&path, br#"{"version": 99, "tasks": []}"#).unwrap();
        assert!(matches!(
            load(&path),
            Err(StoreError::UnsupportedVersion(99))
        ));
    }

    #[test]
    fn save_is_atomic_and_leaves_no_temp_file() {
        let dir = temp_dir("atomic");
        let path = dir.join("reminders.json");
        let mut store = Store::default();
        store.tasks.push(Task::new(
            "1",
            "x",
            Schedule::Countdown {
                countdown_minutes: 5,
            },
        ));
        save(&path, &store).unwrap();
        save(&path, &store).unwrap();
        assert!(path.exists());
        // No scratch file may survive a successful save, whatever its name.
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "temp files left behind: {leftovers:?}");
    }

    #[test]
    fn empty_file_is_treated_as_default() {
        let dir = temp_dir("empty");
        let path = dir.join("reminders.json");
        fs::write(&path, b"").unwrap();
        assert!(load(&path).unwrap().tasks.is_empty());
    }

    #[test]
    fn config_path_uses_appdata_layout() {
        let p = config_path("LocalTools", "WorkReminder");
        let s = p.to_string_lossy().replace('\\', "/");
        assert!(s.ends_with("LocalTools/WorkReminder/reminders.json"), "got {}", s);
    }

    #[test]
    fn save_succeeds_on_a_second_call_over_an_existing_file() {
        // Regression: `save` once opened the temp file a second time read-only just to
        // fsync it. On Windows that open is denied (os error 5), so every save after
        // the first returned an error and the UI reported a failure it could not
        // explain. A repeat save is the cheapest way to cover that path.
        let dir = temp_dir("resave");
        let path = dir.join("reminders.json");
        let mut store = Store::default();
        store.tasks.push(Task::new(
            "1",
            "x",
            Schedule::Countdown {
                countdown_minutes: 5,
            },
        ));
        save(&path, &store).expect("first save");
        store.tasks.push(Task::new(
            "2",
            "y",
            Schedule::Countdown {
                countdown_minutes: 7,
            },
        ));
        save(&path, &store).expect("second save over an existing file");

        // The second save must have replaced the file, not appended to it.
        let reloaded = load(&path).expect("reload");
        assert_eq!(reloaded.tasks.len(), 2);
    }

    #[test]
    fn concurrent_saves_do_not_share_a_temp_file() {
        // Each writer must use its own scratch name; a fixed name lets one process
        // truncate the other's half-written payload and move a torn file into place.
        let dir = temp_dir("concurrent");
        let path = dir.join("reminders.json");
        let mut store = Store::default();
        store.tasks.push(Task::new(
            "1",
            "x",
            Schedule::Countdown {
                countdown_minutes: 5,
            },
        ));

        let path_a = path.clone();
        let store_a = store.clone();
        let a = std::thread::spawn(move || save(&path_a, &store_a));
        let path_b = path.clone();
        let b = std::thread::spawn(move || save(&path_b, &store_b_clone(&store)));
        a.join().unwrap().expect("thread a save");
        b.join().unwrap().expect("thread b save");
        assert!(load(&path).is_ok(), "file must remain parseable");
    }

    fn store_b_clone(s: &Store) -> Store {
        let mut c = Store::default();
        c.tasks = s.tasks.clone();
        c
    }

}
