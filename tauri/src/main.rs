// Copyright (c) 2026 Ma Xin <maxin.zhang@users.noreply.github.com>
// SPDX-License-Identifier: MIT
//
// Tauri front end for the reminder engine.
//
// Division of labour:
//   core/       scheduling, persistence, time arithmetic — no platform calls, unit
//               tested independently, and shared verbatim with the Win32 build
//   this file   owns the timer, talks to the OS (tray, autostart, chime), and
//               exposes everything to the UI as `#[tauri::command]` calls
//   ui/         presentation only: no timer, no storage, no scheduling
//
// The UI never computes a due time. It asks for a snapshot and renders it, which
// keeps the twelve schedule types in one place instead of two.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, State, WindowEvent,
};

use reminder_core::engine::{Engine, FiringKind};
use reminder_core::model::{PomodoroState, Schedule, ScheduleType, Task};
use reminder_core::store::{self, Store};
use reminder_core::time::{self, DateTime};

/// Tick resolution. One second keeps short schedules (countdown, pomodoro phases)
/// honest without burning CPU: a tick costs microseconds at a thousand tasks.
const TICK_MS: u64 = 1000;

const APP_NAME: &str = "工作提醒助手";

/// Everything the commands touch, behind one lock.
///
/// A single mutex rather than several: the tick holds it for microseconds, and one
/// coarse lock avoids the "lock A then B" deadlock class entirely.
struct AppState {
    engine: Engine,
    tasks: Vec<Task>,
    path: PathBuf,
    /// Set when the saved file could not be read, so the UI can say so instead of
    /// silently starting empty and looking like data loss.
    load_error: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    tasks: Vec<TaskView>,
    /// Wall clock at snapshot time, so the UI renders relative times from the same
    /// clock the engine used instead of the webview's own.
    now: String,
    load_error: Option<String>,
}

/// A task as the UI needs it: stored fields plus the derived strings the list
/// shows. Deriving here means the UI never re-implements `describe()`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TaskView {
    id: String,
    title: String,
    message: String,
    schedule: Schedule,
    /// Kind token, so the edit form can preselect without reverse-engineering.
    kind: String,
    schedule_text: String,
    next_text: String,
    enabled: bool,
    overdue: bool,
    pomodoro: PomodoroState,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FiredTask {
    id: String,
    title: String,
    message: String,
    /// Only for `UsageBreak` wording; zero for every other kind.
    usage_seconds: i64,
    pre_alert: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SaveTask {
    /// Empty when creating; the backend assigns an id.
    id: String,
    title: String,
    message: String,
    kind: String,
    params: Vec<String>,
    enabled: bool,
}

fn view_of(task: &Task, now: &DateTime) -> TaskView {
    let next_text = match (task.enabled, task.next_trigger) {
        (false, _) => "—".to_string(),
        (true, None) => "未排定".to_string(),
        (true, Some(t)) if t < *now => "已过期".to_string(),
        (true, Some(t)) => format!(
            "{:02}-{:02} {:02}:{:02}",
            t.date.month, t.date.day, t.time.hour, t.time.minute
        ),
    };
    TaskView {
        id: task.id.clone(),
        title: task.title.clone(),
        message: task.message.clone(),
        schedule: task.schedule.clone(),
        kind: task.schedule_type().as_token().to_string(),
        schedule_text: task.describe(),
        next_text,
        enabled: task.enabled,
        overdue: task.enabled && task.next_trigger.map(|t| t < *now).unwrap_or(false),
        pomodoro: task.pomodoro.clone(),
    }
}

fn snapshot(st: &AppState) -> Snapshot {
    let now = time::now_local();
    Snapshot {
        tasks: st.tasks.iter().map(|t| view_of(t, &now)).collect(),
        now: format!(
            "{:04}-{:02}-{:02} {:02}:{:02}",
            now.date.year, now.date.month, now.date.day, now.time.hour, now.time.minute
        ),
        load_error: st.load_error.clone(),
    }
}

/// Write the store, reporting failure instead of swallowing it.
///
/// A GUI build has no console, so `eprintln!` on a failed save was invisible: the
/// list looked saved while nothing reached disk. Return the error so every command
/// can surface it, and log it for the case where nobody is looking.
fn persist(st: &AppState) -> Result<(), String> {
    let store = Store {
        version: store::SCHEMA_VERSION,
        tasks: st.tasks.clone(),
    };
    match store::save(&st.path, &store) {
        Ok(()) => Ok(()),
        Err(e) => {
            let msg = format!("保存失败：{e}（{}）", st.path.display());
            trace(&msg);
            Err(msg)
        }
    }
}

/// Advance the engine once and return what fired.
fn advance(st: &mut AppState) -> Vec<FiredTask> {
    let now = time::now_local();
    let mut out = Vec::new();
    // `tick` borrows tasks mutably, so build the payloads before releasing them.
    {
        let firings = st.engine.tick(&mut st.tasks, now, idle_seconds());
        for f in firings {
            out.push(FiredTask {
                id: f.task.id.clone(),
                title: f.task.title.clone(),
                message: f.task.message.clone(),
                usage_seconds: f.usage_seconds,
                pre_alert: matches!(f.kind, FiringKind::PreAlert),
            });
        }
    }
    if !out.is_empty() {
        let _ = persist(st);
    }
    out
}

// ------------------------------------------------------------------ commands

#[tauri::command]
fn ui_log(message: String) {
    trace(&format!("ui: {message}"));
}

#[tauri::command]
fn get_snapshot(state: State<'_, Mutex<AppState>>) -> Snapshot {
    snapshot(&lock(&state))
}

#[tauri::command]
fn describe_kinds() -> Vec<(String, String)> {
    ScheduleType::ALL
        .iter()
        .map(|k| (k.as_token().to_string(), k.label().to_string()))
        .collect()
}

/// Parameter inputs for a kind, as (label, hint, default).
///
/// The default is what the form pre-fills. Leaving a field blank looks harmless but
/// turns into a validation error at save time, which is a needless round trip for
/// the common case (a 60-minute interval, 09:00, and so on).
#[tauri::command]
fn param_labels(kind: String) -> Vec<(String, String, String)> {
    let now = time::now_local();
    ScheduleType::from_token(&kind)
        .map(|k| {
            param_fields(k)
                .into_iter()
                .map(|f| {
                    let def = if f.default == "@now+60@" {
                        // "In an hour" is the answer people actually want for a
                        // one-off reminder, and a blank field here is a save error.
                        now.add_minutes(60).format_ymd_hm()
                    } else {
                        f.default.to_string()
                    };
                    (f.label.to_string(), f.hint.to_string(), def)
                })
                .collect()
        })
        .unwrap_or_default()
}

#[tauri::command]
fn save_task(state: State<'_, Mutex<AppState>>, task: SaveTask) -> Result<Snapshot, String> {
    let mut st = lock(&state);
    let title = task.title.trim().to_string();
    if title.is_empty() {
        return Err("请填写任务名称".into());
    }
    let kind = ScheduleType::from_token(&task.kind)
        .ok_or_else(|| format!("未知的提醒方式：{}", task.kind))?;
    let schedule = build_schedule(kind, &task.params)?;
    let now = time::now_local();

    if task.id.is_empty() {
        let mut t = Task::new(new_id(&st.tasks), title, schedule);
        t.message = task.message.trim().to_string();
        t.enabled = task.enabled;
        t.next_trigger = reminder_core::scheduler::next_trigger(&t.schedule, now, now);
        st.tasks.push(t);
    } else {
        let slot = st
            .tasks
            .iter_mut()
            .find(|t| t.id == task.id)
            .ok_or_else(|| "任务不存在，可能已被删除".to_string())?;
        slot.title = title;
        slot.message = task.message.trim().to_string();
        slot.schedule = schedule;
        slot.enabled = task.enabled;
        // Re-arm from now: the user expects the next occurrence, not a burst of
        // catch-up fires for every occurrence that passed while editing.
        slot.next_trigger = reminder_core::scheduler::next_trigger(&slot.schedule, now, now);
    }
    persist(&st)?;
    Ok(snapshot(&st))
}

#[tauri::command]
fn set_enabled(state: State<'_, Mutex<AppState>>, id: String, enabled: bool) -> Snapshot {
    let mut st = lock(&state);
    let now = time::now_local();
    if let Some(t) = st.tasks.iter_mut().find(|t| t.id == id) {
        t.enabled = enabled;
        if enabled {
            t.next_trigger = reminder_core::scheduler::next_trigger(&t.schedule, now, now);
        }
    }
    let _ = persist(&st);
    snapshot(&st)
}

#[tauri::command]
fn delete_task(state: State<'_, Mutex<AppState>>, id: String) -> Snapshot {
    let mut st = lock(&state);
    st.tasks.retain(|t| t.id != id);
    let _ = persist(&st);
    snapshot(&st)
}

/// Advance the engine now. The UI calls this after a save so the list updates
/// without waiting up to one second for the timer.
#[tauri::command]
fn tick_now(state: State<'_, Mutex<AppState>>) -> Vec<FiredTask> {
    advance(&mut lock(&state))
}

#[tauri::command]
fn set_autostart(app: AppHandle, enabled: bool) -> Result<bool, String> {
    use tauri_plugin_autostart::ManagerExt;
    let mgr = app.autolaunch();
    let r = if enabled { mgr.enable() } else { mgr.disable() };
    r.map_err(|e| format!("{e}"))?;
    mgr.is_enabled().map_err(|e| format!("{e}"))
}

#[tauri::command]
fn get_autostart(app: AppHandle) -> bool {
    use tauri_plugin_autostart::ManagerExt;
    app.autolaunch().is_enabled().unwrap_or(false)
}

/// Un-poison a mutex: a panic in one command must not take the whole app down,
/// because the state it guards is still structurally sound.
fn lock<'a, 'b>(state: &'b State<'a, Mutex<AppState>>) -> std::sync::MutexGuard<'b, AppState> {
    match state.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

// ------------------------------------------------------- schedule building

struct ParamField {
    label: &'static str,
    hint: &'static str,
    /// Pre-filled value: the answer most users would give.
    default: &'static str,
}

/// Which parameter inputs a kind needs, in display order.
///
/// One table drives both the form and the parser, so a field can never appear in
/// the UI without the backend knowing how to read it.
fn param_fields(kind: ScheduleType) -> Vec<ParamField> {
    use ScheduleType::*;
    let f = |label, hint, default| ParamField { label, hint, default };
    match kind {
        Interval => vec![f("间隔", "分钟，例如 60", "60")],
        Daily => vec![f("时间", "HH:MM，例如 09:00", "09:00")],
        // A concrete default is produced at request time; see `@now+60@`.
        Once => vec![f("时间", "YYYY-MM-DD HH:MM", "@now+60@")],
        Weekly => vec![
            f("星期", "1-7 逗号分隔，例如 1,3,5", "1,3,5"),
            f("时间", "HH:MM，例如 09:00", "09:00"),
        ],
        Workdays => vec![f("时间", "HH:MM，例如 09:00", "09:00")],
        Monthly => vec![
            f("日期", "1-31 / last / lastwork", "lastwork"),
            f("时间", "HH:MM，例如 09:00", "09:00"),
        ],
        MultiDaily => vec![f("时间列表", "逗号分隔，例如 09:00,12:00,17:30", "09:00,12:00,17:30")],
        TimedInterval => vec![
            f("开始", "HH:MM", "09:00"),
            f("结束", "HH:MM", "18:00"),
            f("间隔", "分钟", "60"),
        ],
        Countdown => vec![f("分钟", "例如 25", "25")],
        Pomodoro => vec![
            f("专注", "分钟，例如 25", "25"),
            f("休息", "分钟，例如 5", "5"),
            f("轮数", "例如 4", "4"),
        ],
        AfterStartup => vec![f("延迟", "分钟", "5")],
        UsageBreak => vec![f("连续使用", "分钟，例如 45", "45")],
    }
}

fn parse_hhmm(text: &str) -> Result<time::TimeOfDay, String> {
    let t = text.trim();
    let (h, m) = t
        .split_once(':')
        .ok_or_else(|| format!("时间格式应为 HH:MM，收到：{t}"))?;
    let hour: u32 = h.trim().parse().map_err(|_| format!("小时不是数字：{h}"))?;
    let minute: u32 = m.trim().parse().map_err(|_| format!("分钟不是数字：{m}"))?;
    if hour > 23 || minute > 59 {
        return Err(format!("时间超出范围：{t}"));
    }
    Ok(time::TimeOfDay::new(hour, minute, 0))
}

fn parse_minutes(text: &str, what: &str) -> Result<i64, String> {
    let t = text.trim();
    let v: i64 = t
        .parse()
        .map_err(|_| format!("{what}应为整数分钟，收到：{t}"))?;
    if v <= 0 {
        return Err(format!("{what}必须大于 0"));
    }
    Ok(v)
}

fn build_schedule(kind: ScheduleType, params: &[String]) -> Result<Schedule, String> {
    use ScheduleType::*;
    let get = |i: usize| params.get(i).map(|s| s.as_str()).unwrap_or("");

    Ok(match kind {
        Interval => Schedule::Interval {
            interval_minutes: parse_minutes(get(0), "间隔")?,
        },
        Daily => Schedule::Daily {
            daily_time: parse_hhmm(get(0))?,
        },
        Once => Schedule::Once {
            once_at: time::DateTime::parse_ymd_hm(get(0))
                .ok_or_else(|| format!("无法解析时间：{}", get(0)))?,
        },
        Weekly => Schedule::Weekly {
            weekdays_mask: parse_weekdays(get(0))?,
            daily_time: parse_hhmm(get(1))?,
        },
        Workdays => Schedule::Workdays {
            daily_time: parse_hhmm(get(0))?,
        },
        Monthly => {
            let (monthly_mode, monthly_day) = parse_monthly(get(0))?;
            Schedule::Monthly {
                monthly_mode,
                monthly_day,
                daily_time: parse_hhmm(get(1))?,
            }
        }
        MultiDaily => {
            let mut daily_times = Vec::new();
            for part in get(0).split(',') {
                if !part.trim().is_empty() {
                    daily_times.push(parse_hhmm(part)?);
                }
            }
            if daily_times.is_empty() {
                return Err("至少需要一个时间".into());
            }
            Schedule::MultiDaily { daily_times }
        }
        TimedInterval => Schedule::TimedInterval {
            weekdays_mask: reminder_core::model::WORKDAY_MASK,
            active_start: parse_hhmm(get(0))?,
            active_end: parse_hhmm(get(1))?,
            interval_minutes: parse_minutes(get(2), "间隔")?,
        },
        Countdown => Schedule::Countdown {
            countdown_minutes: parse_minutes(get(0), "倒计时")?,
        },
        Pomodoro => Schedule::Pomodoro {
            work_minutes: parse_minutes(get(0), "专注时长")?,
            break_minutes: parse_minutes(get(1), "休息时长")?,
            cycles: get(2)
                .trim()
                .parse()
                .map_err(|_| "轮数应为整数".to_string())?,
        },
        AfterStartup => Schedule::AfterStartup {
            startup_delay_minutes: parse_minutes(get(0), "延迟")?,
        },
        UsageBreak => Schedule::UsageBreak {
            usage_minutes: parse_minutes(get(0), "连续使用时长")?,
        },
    })
}

fn parse_weekdays(text: &str) -> Result<u8, String> {
    let t = text.trim();
    if t.is_empty() {
        return Err("请填写星期".into());
    }
    let mut mask = 0u8;
    for part in t.split(',') {
        let v: u32 = part
            .trim()
            .parse()
            .map_err(|_| format!("星期应为 1-7：{part}"))?;
        if !(1..=7).contains(&v) {
            return Err(format!("星期超出范围：{v}"));
        }
        mask |= 1 << (v - 1);
    }
    Ok(mask)
}

fn parse_monthly(text: &str) -> Result<(reminder_core::model::MonthlyMode, u32), String> {
    use reminder_core::model::MonthlyMode;
    let t = text.trim().to_ascii_lowercase();
    Ok(match t.as_str() {
        "last" => (MonthlyMode::LastDay, 0),
        "lastwork" => (MonthlyMode::LastWorkday, 0),
        other => {
            let day: u32 = other
                .parse()
                .map_err(|_| format!("日期应为 1-31 / last / lastwork：{other}"))?;
            if !(1..=31).contains(&day) {
                return Err(format!("日期超出范围：{day}"));
            }
            (MonthlyMode::Day, day)
        }
    })
}

fn new_id(tasks: &[Task]) -> String {
    // Collision-free against the current set, and readable in the file.
    let mut seq = tasks.len() as u64 + 1;
    loop {
        let id = format!("t{seq}");
        if !tasks.iter().any(|t| t.id == id) {
            return id;
        }
        seq += 1;
    }
}

// ------------------------------------------------------------------- OS glue

/// Seconds since the last keyboard or mouse input, system-wide.
///
/// `UsageBreak` schedules measure continuous usage, so this has to come from the OS.
/// Passing a constant 0 instead means the idle reset never triggers and the counter
/// climbs during sleep, meetings and lunch — a break reminder that fires only after
/// hours away from the desk is worse than none.
///
/// Returns 0 when the query fails, which biases towards "user is present": a failed
/// query must never fire a break.
#[cfg(windows)]
fn idle_seconds() -> i64 {
    #[repr(C)]
    struct LastInputInfo {
        cb_size: u32,
        time: u32,
    }
    #[link(name = "user32")]
    extern "system" {
        fn GetLastInputInfo(info: *mut LastInputInfo) -> i32;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetTickCount() -> u32;
    }

    let mut info = LastInputInfo {
        cb_size: std::mem::size_of::<LastInputInfo>() as u32,
        time: 0,
    };
    // SAFETY: `info` is a correctly sized out-parameter.
    if unsafe { GetLastInputInfo(&mut info) } == 0 {
        return 0;
    }
    // SAFETY: GetTickCount takes no arguments and cannot fail.
    let now = unsafe { GetTickCount() };
    // Wrapping subtraction is intentional: the counter is 32-bit and rolls over
    // every ~49.7 days, and wrapping keeps the delta correct across the rollover.
    let delta_ms = now.wrapping_sub(info.time) as i64;
    delta_ms / 1000
}

#[cfg(not(windows))]
fn idle_seconds() -> i64 {
    0
}

/// Two short tones. A reminder that only appears visually gets missed while the
/// user is in another window.
#[cfg(windows)]
fn play_chime() {
    #[link(name = "kernel32")]
    extern "system" {
        fn Beep(freq: u32, dur: u32) -> i32;
    }
    // SAFETY: Beep only needs two integers; no handles involved.
    unsafe {
        Beep(880, 90);
        Beep(1170, 110);
    }
}

#[cfg(not(windows))]
fn play_chime() {}

fn config_path() -> PathBuf {
    // Same directory as the Win32 build, so an existing reminders.json keeps
    // working: switching front ends must not look like data loss.
    store::config_path("MaXin", "WorkReminder")
}

// ---------------------------------------------------------------- lifecycle


/// Append a line to `<exe dir>/work-reminder-YYYY-MM-DD.log`.
///
/// A GUI binary has no console, so a failure before the window exists is otherwise
/// invisible. This is deliberate, permanent instrumentation for that startup window
/// where no UI can report anything.
///
/// The file name carries the local date, which rotates the log daily without a
/// dependency: yesterday's file is simply never opened again. `%TEMP%` can be
/// redirected or unwritable under a restricted token, so the executable's own
/// directory is tried first; a diagnostic that silently fails is not a diagnostic.
fn trace(msg: &str) {
    use std::io::Write;

    // Local date from the same clock the engine uses, formatted once per call.
    let stamp = time::now_local();
    let day = format!(
        "{:04}-{:02}-{:02}",
        stamp.date.year, stamp.date.month, stamp.date.day
    );
    let line = format!(
        "[{} {:02}:{:02}:{:02}] {msg}",
        day, stamp.time.hour, stamp.time.minute, stamp.time.second
    );

    let mut bases = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            bases.push(dir.to_path_buf());
        }
    }
    bases.push(std::env::temp_dir());
    for base in bases {
        let target = base.join(format!("work-reminder-{day}.log"));
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&target)
        {
            let _ = writeln!(f, "{line}");
            let _ = f.flush();
            return;
        }
    }
}

fn main() {
    trace("start");
    if std::env::args().any(|a| a == "--probe") {
        trace("probe mode, exiting before tauri");
        return;
    }
    // `--params <kind>` prints the parameter table for one kind, so defaults can be
    // checked without driving the webview.
    if let Some(kind) = std::env::args().position(|a| a == "--params") {
        if let Some(name) = std::env::args().nth(kind + 1) {
            if let Some(stype) = ScheduleType::from_token(&name) {
                let now = time::now_local();
                for f in param_fields(stype) {
                    let def = if f.default == "@now+60@" {
                        now.add_minutes(60).format_ymd_hm()
                    } else {
                        f.default.to_string()
                    };
                    println!("{}\t{}\t{}", f.label, f.hint, def);
                }
                return;
            }
            eprintln!("unknown kind: {name}");
            return;
        }
    }
    let path = config_path();
    trace(&format!("config path {}", path.display()));
    // `StoreError` has no NotFound variant: a missing file is only known before the
    // read, so decide that here rather than treating every IO error as data loss.
    let (mut tasks, load_error) = if !path.exists() {
        (Vec::new(), None)
    } else {
        match store::load(&path) {
            Ok(s) => (s.tasks, None),
            Err(e) => (Vec::new(), Some(format!("读取配置失败：{e}"))),
        }
    };

    // Arm anything that was never scheduled, so a hand-edited file still works.
    let now = time::now_local();
    for t in tasks.iter_mut() {
        if t.next_trigger.is_none() {
            t.next_trigger = reminder_core::scheduler::next_trigger(&t.schedule, now, now);
        }
    }

    let mut engine = Engine::new(now);
    engine.retain_known(&tasks);

    // `--selftest=<name>` drives one scripted interaction in the UI. It exists because
    // injected mouse events never reach WebView2 content, so this is the only way to
    // exercise render -> form -> IPC -> disk from an automated check. Debug builds
    // only: a release binary must not be remotely drivable.
    let selftest = if cfg!(debug_assertions) {
        std::env::args().find_map(|a| a.strip_prefix("--selftest=").map(str::to_string))
    } else {
        None
    };
    if let Some(name) = &selftest {
        trace(&format!("selftest requested: {name}"));
    }

    trace("state ready, building app");
    tauri::Builder::default()
        // Registered first so the check runs before any window, tray or timer exists.
        // Without it a second launch starts a rival scheduler writing the same file —
        // two processes racing on one JSON, each with its own timer.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            trace("second launch detected, focusing the running window");
            focus_main(app);
        }))
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .manage(Mutex::new(AppState {
            engine,
            tasks,
            path,
            load_error,
        }))
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            describe_kinds,
            param_labels,
            save_task,
            set_enabled,
            delete_task,
            tick_now,
            set_autostart,
            get_autostart,
            ui_log,
        ])
        .setup(move |app| {
            if let Some(name) = selftest.clone() {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.eval(&format!(
                        "setTimeout(() => window.__runSelfTest && window.__runSelfTest({name:?}), 600)"
                    ));
                }
            }
            trace("setup: building tray");
            match build_tray(app.handle()) {
                Ok(()) => trace("setup: tray ok"),
                Err(e) => trace(&format!("setup: tray FAILED {e}")),
            }
            start_ticker(app.handle().clone());
            trace("setup: done");
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing hides: a tray reminder must keep working after the user is
            // done with the list.
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("failed to start the application");
    trace("message loop returned");
}

fn start_ticker(app: AppHandle) {
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_millis(TICK_MS));
        let state = app.state::<Mutex<AppState>>();
        // Catch panics from the tick: with `panic = "abort"` in release a panic
        // here would kill the whole process — tray, reminders and all — because
        // of one bad schedule value. A tick that panics is skipped and logged
        // instead; the state it guards stays structurally sound.
        let fired = {
            let mut st = match state.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| advance(&mut st))) {
                Ok(fired) => fired,
                Err(_) => {
                    trace("tick panicked; tick skipped, state preserved");
                    continue;
                }
            }
        };
        if fired.is_empty() {
            continue;
        }
        if fired.iter().any(|f| !f.pre_alert) {
            play_chime();
        }
        // One payload per batch: three reminders in the same second should stack
        // in the UI, not trigger three re-renders.
        let _ = app.emit("reminders-fired", &fired);
    });
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "打开任务列表", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;

    let mut builder = TrayIconBuilder::with_id("main")
        .tooltip(APP_NAME)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => focus_main(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                focus_main(tray.app_handle());
            }
        });
    // The tray needs an icon; fall back to the window icon rather than a blank
    // slot, which some Windows versions render as an empty square.
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

fn focus_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}
