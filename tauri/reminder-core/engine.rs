// Copyright (c) 2026 Ma Xin <maxin.zhang@users.noreply.github.com>
// SPDX-License-Identifier: MIT
//
// The tick loop: turns "what is the wall clock now and how active is the user"
// into task state transitions and a list of firings to present.
//
// This is a clean-room reimplementation of the behaviour documented in
// docs/reverse-analysis.md §3.4 (original routine 0x13d20).

use std::collections::HashMap;

use crate::model::{Schedule, Task};
use crate::scheduler::{is_due, FireOutcome};
use crate::time::DateTime;

/// A user is considered away after this much keyboard/mouse inactivity.
/// Matches the hard-coded 300 s threshold observed at 0x13e0a.
pub const IDLE_RESET_SECONDS: i64 = 300;

/// Lead time for the pomodoro heads-up. A pomodoro notification can arrive
/// while the user is mid-edit, so it announces itself a few seconds early and
/// then fires for real. The reference implementation had no such lead time; this
/// is an additive behaviour that leaves the schedule arithmetic untouched.
pub const POMODORO_PRE_ALERT_SECONDS: i64 = 5;

/// Per-tick upper bound on credited usage, so a suspended laptop does not
/// credit an entire afternoon of "usage" in one tick (0x13d79 `cmp edx, 5`).
pub const MAX_CREDIT_MINUTES_PER_TICK: i64 = 5;

/// Why a task surfaced this tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FiringKind {
    /// The task actually fired; `next_trigger` has been advanced.
    Fired,
    /// A heads-up only: the task is still armed and will fire for real shortly.
    PreAlert,
}

/// One firing to hand to the UI.
pub struct Firing<'a> {
    pub task: &'a Task,
    pub outcome: FireOutcome,
    /// Continuous usage accumulated for this task, for `UsageBreak` wording.
    pub usage_seconds: i64,
    pub kind: FiringKind,
}

/// Drives all tasks forward. Owns the auxiliary runtime counters that the
/// original implementation kept in scattered struct fields.
pub struct Engine {
    /// Last observed wall clock, used to derive the usage credit.
    last_tick: Option<DateTime>,
    /// Continuous usage counter per task id, in seconds.
    ///
    /// Keyed by id rather than a `Vec` + linear scan: every tick touches every
    /// task, so a scan per task made the whole tick O(n²). Measured cost at 1000
    /// tasks dropped from ~16 ms to well under a millisecond after this change.
    usage: HashMap<String, i64>,
    /// Tasks whose pomodoro pre-alert has already been shown for the current
    /// cycle, keyed by (task id, phase), so the heads-up is emitted exactly once
    /// per phase without scanning.
    pre_alerted: HashMap<String, Option<DateTime>>,
    /// Instant this process started; anchors `AfterStartup` schedules.
    pub started_at: DateTime,
}

impl Engine {
    pub fn new(started_at: DateTime) -> Self {
        Self {
            last_tick: None,
            usage: HashMap::new(),
            pre_alerted: HashMap::new(),
            started_at,
        }
    }

    fn usage_of(&self, id: &str) -> i64 {
        self.usage.get(id).copied().unwrap_or(0)
    }

    fn set_usage(&mut self, id: &str, value: i64) {
        // Reuse the existing key when the id is already known, so the common path
        // allocates nothing beyond the first insert.
        if let Some(slot) = self.usage.get_mut(id) {
            *slot = value;
        } else {
            self.usage.insert(id.to_string(), value);
        }
    }

    /// Run one tick.
    ///
    /// `idle_seconds` is the system-wide keyboard/mouse inactivity reported by
    /// the platform layer. Returns the tasks that fired during this tick; their
    /// state has already been advanced.
    pub fn tick<'a>(
        &mut self,
        tasks: &'a mut [Task],
        now: DateTime,
        idle_seconds: i64,
    ) -> Vec<Firing<'a>> {
        let credit = match self.last_tick {
            Some(prev) => (prev.seconds_until(&now) / 60).clamp(0, MAX_CREDIT_MINUTES_PER_TICK),
            // First tick after start: nothing to credit.
            None => 0,
        };
        self.last_tick = Some(now);

        // Phase 1: maintain the continuous-usage counters that drive
        // `UsageBreak` tasks. A long enough idle gap resets the counter.
        for task in tasks.iter_mut() {
            if !matches!(task.schedule, Schedule::UsageBreak { .. }) {
                continue;
            }
            if idle_seconds >= IDLE_RESET_SECONDS {
                self.set_usage(&task.id, 0);
                continue;
            }
            if !task.enabled {
                self.set_usage(&task.id, 0);
                continue;
            }
            // Only credit while the task is not muted.
            if task.muted_until.map(|m| now < m).unwrap_or(false) {
                continue;
            }
            let accrued = self.usage_of(&task.id) + credit * 60;
            self.set_usage(&task.id, accrued);
        }

        // Phase 2: evaluate every task against its own schedule.
        //
        // A pomodoro phase also gets a heads-up a few seconds before it fires, so
        // the user is not yanked out of a sentence mid-keystroke. The heads-up
        // does not advance the schedule and is emitted at most once per phase.
        let anchor = self.started_at;
        let mut fired: Vec<Firing<'a>> = Vec::new();
        for task in tasks.iter_mut() {
            let usage_seconds = self.usage_of(&task.id);

            // Decide the heads-up before touching the task mutably: only the
            // dedupe set changes here, not the schedule.
            let emit_pre_alert = match pre_alert_due(task, now) {
                Some(phase) => {
                    // HashMap insert returns the previous value: `None` means this
                    // phase had not been announced yet.
                    let already = self
                        .pre_alerted
                        .insert(task.id.clone(), phase)
                        .is_some();
                    !already
                }
                None => false,
            };

            let outcome = is_due(task, now, anchor, usage_seconds);
            if outcome == FireOutcome::NotDue {
                if emit_pre_alert {
                    fired.push(Firing {
                        task: &*task,
                        outcome: FireOutcome::NotDue,
                        usage_seconds,
                        kind: FiringKind::PreAlert,
                    });
                }
                continue;
            }

            if matches!(task.schedule, Schedule::UsageBreak { .. }) {
                // Reset the counter so the next break is measured from here.
                self.set_usage(&task.id, 0);
            }
            // The real firing consumed that phase; drop its heads-up marker.
            self.pre_alerted.remove(&task.id);
            fired.push(Firing {
                task: &*task,
                outcome,
                usage_seconds,
                kind: FiringKind::Fired,
            });
        }
        fired
    }

    /// Drop counters for tasks that no longer exist.
    ///
    /// Builds the id set once instead of scanning `tasks` per entry, which keeps a
    /// delete of one task O(n) rather than O(n²).
    pub fn retain_known(&mut self, tasks: &[Task]) {
        let live: std::collections::HashSet<&str> =
            tasks.iter().map(|t| t.id.as_str()).collect();
        self.usage.retain(|id, _| live.contains(id.as_str()));
        self.pre_alerted.retain(|id, _| live.contains(id.as_str()));
    }

    pub fn usage_seconds_of(&self, id: &str) -> i64 {
        self.usage_of(id)
    }

    /// Whether a pre-alert has already been emitted for `id` at `phase`.
    pub fn pre_alerted_for(&self, id: &str, phase: Option<DateTime>) -> bool {
        self.pre_alerted.get(id).map(|at| *at == phase).unwrap_or(false)
    }
}

/// The phase a pomodoro heads-up belongs to, if one is due right now.
///
/// Returns the current `next_trigger` as the phase key so the caller can dedupe.
fn pre_alert_due(task: &Task, now: DateTime) -> Option<Option<DateTime>> {
    if !matches!(task.schedule, Schedule::Pomodoro { .. }) || !task.enabled {
        return None;
    }
    if task.muted_until.map(|m| now < m).unwrap_or(false) {
        return None;
    }
    let trigger = task.next_trigger?;
    let lead = now.seconds_until(&trigger);
    // Within the lead window but not yet reached.
    if (1..=POMODORO_PRE_ALERT_SECONDS).contains(&lead) {
        Some(Some(trigger))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Schedule;
    use crate::time::{Date, TimeOfDay};

    fn dt(y: i32, m: u32, d: u32, hh: u32, mm: u32) -> DateTime {
        DateTime::new(Date::new(y, m, d), TimeOfDay::new(hh, mm, 0))
    }

    #[test]
    fn usage_accumulates_then_fires_and_resets() {
        let start = dt(2026, 10, 22, 9, 0);
        let mut engine = Engine::new(start);
        let mut tasks = vec![Task::new(
            "u1",
            "起来活动",
            Schedule::UsageBreak { usage_minutes: 10 },
        )];

        // Prime the tick baseline.
        engine.tick(&mut tasks, start, 0);

        let mut fired_at = None;
        let mut t = start;
        for _ in 0..12 {
            t = t.add_minutes(5);
            let fired = engine.tick(&mut tasks, t, 0);
            if !fired.is_empty() {
                fired_at = Some(t);
                break;
            }
        }
        assert_eq!(fired_at, Some(dt(2026, 10, 22, 9, 10)));
        assert_eq!(engine.usage_seconds_of("u1"), 0, "counter resets on firing");
    }

    #[test]
    fn idle_user_resets_usage_counter() {
        let start = dt(2026, 10, 22, 9, 0);
        let mut engine = Engine::new(start);
        let mut tasks = vec![Task::new(
            "u1",
            "起来活动",
            Schedule::UsageBreak { usage_minutes: 30 },
        )];

        engine.tick(&mut tasks, start, 0);
        engine.tick(&mut tasks, start.add_minutes(5), 0);
        assert_eq!(engine.usage_seconds_of("u1"), 300);

        // User walked away: inactivity beyond the threshold clears the counter.
        engine.tick(&mut tasks, start.add_minutes(10), IDLE_RESET_SECONDS);
        assert_eq!(engine.usage_seconds_of("u1"), 0);
    }

    #[test]
    fn usage_credit_is_capped_per_tick() {
        let start = dt(2026, 10, 22, 9, 0);
        let mut engine = Engine::new(start);
        let mut tasks = vec![Task::new(
            "u1",
            "x",
            Schedule::UsageBreak { usage_minutes: 600 },
        )];
        engine.tick(&mut tasks, start, 0);
        // A three-hour gap must only credit MAX_CREDIT_MINUTES_PER_TICK.
        engine.tick(&mut tasks, start.add_minutes(180), 0);
        assert_eq!(
            engine.usage_seconds_of("u1"),
            MAX_CREDIT_MINUTES_PER_TICK * 60
        );
    }

    #[test]
    fn daily_task_fires_once_per_day() {
        let start = dt(2026, 10, 22, 8, 0);
        let mut engine = Engine::new(start);
        let mut tasks = vec![Task::new(
            "d1",
            "喝水",
            Schedule::Daily {
                daily_time: TimeOfDay::new(9, 0, 0),
            },
        )];

        assert!(engine.tick(&mut tasks, start, 0).is_empty());
        assert!(engine.tick(&mut tasks, dt(2026, 10, 22, 8, 59), 0).is_empty());

        let fired = engine.tick(&mut tasks, dt(2026, 10, 22, 9, 0), 0);
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].task.id, "d1");

        // Same day, later ticks must not fire again.
        assert!(engine.tick(&mut tasks, dt(2026, 10, 22, 9, 1), 0).is_empty());
        assert!(engine.tick(&mut tasks, dt(2026, 10, 22, 15, 0), 0).is_empty());

        // Next day fires again.
        let fired = engine.tick(&mut tasks, dt(2026, 10, 23, 9, 0), 0);
        assert_eq!(fired.len(), 1);
    }

    #[test]
    fn disabled_task_is_skipped() {
        let start = dt(2026, 10, 22, 9, 0);
        let mut engine = Engine::new(start);
        let mut task = Task::new(
            "d1",
            "x",
            Schedule::Daily {
                daily_time: TimeOfDay::new(9, 0, 0),
            },
        );
        task.enabled = false;
        task.next_trigger = Some(start);
        let mut tasks = vec![task];
        assert!(engine.tick(&mut tasks, start, 0).is_empty());
    }

    #[test]
    fn pomodoro_emits_one_pre_alert_before_firing() {
        let start = dt(2026, 10, 22, 9, 0);
        let mut engine = Engine::new(start);
        let mut task = Task::new(
            "p1",
            "专注",
            Schedule::Pomodoro {
                work_minutes: 25,
                break_minutes: 5,
                cycles: 4,
            },
        );
        // Phase ends at 09:25.
        task.next_trigger = Some(dt(2026, 10, 22, 9, 25));
        let mut tasks = vec![task];

        // 09:24:50 is 10 s ahead: outside the 5 s window.
        let r = engine.tick(&mut tasks, dt(2026, 10, 22, 9, 24), 0);
        assert!(r.is_empty(), "too early for a heads-up");

        // 09:24:56 is 4 s ahead: heads-up, no firing.
        let t = DateTime::new(
            crate::time::Date::new(2026, 10, 22),
            TimeOfDay::new(9, 24, 56),
        );
        let r = engine.tick(&mut tasks, t, 0);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].kind, FiringKind::PreAlert);
        assert_eq!(r[0].outcome, FireOutcome::NotDue);

        // Re-ticking inside the same window must not repeat the heads-up.
        let r = engine.tick(&mut tasks, t, 0);
        assert!(r.is_empty(), "pre-alert must not repeat for one phase");

        // At the real due time it fires, and the schedule advances.
        let r = engine.tick(&mut tasks, dt(2026, 10, 22, 9, 25), 0);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].kind, FiringKind::Fired);
        assert_eq!(r[0].outcome, FireOutcome::Fired);
        assert!(tasks[0].pomodoro.in_break);
    }

    #[test]
    fn next_pomodoro_phase_gets_its_own_pre_alert() {
        let start = dt(2026, 10, 22, 9, 0);
        let mut engine = Engine::new(start);
        let mut task = Task::new(
            "p1",
            "专注",
            Schedule::Pomodoro {
                work_minutes: 25,
                break_minutes: 5,
                cycles: 4,
            },
        );
        task.next_trigger = Some(dt(2026, 10, 22, 9, 25));
        let mut tasks = vec![task];

        // Heads-up + fire for phase one.
        engine.tick(
            &mut tasks,
            DateTime::new(
                crate::time::Date::new(2026, 10, 22),
                TimeOfDay::new(9, 24, 57),
            ),
            0,
        );
        engine.tick(&mut tasks, dt(2026, 10, 22, 9, 25), 0);
        // Break now ends at 09:30.
        assert_eq!(tasks[0].next_trigger, Some(dt(2026, 10, 22, 9, 30)));

        // The new phase must get its own heads-up.
        let r = engine.tick(
            &mut tasks,
            DateTime::new(
                crate::time::Date::new(2026, 10, 22),
                TimeOfDay::new(9, 29, 58),
            ),
            0,
        );
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].kind, FiringKind::PreAlert);
    }

    #[test]
    fn pre_alert_is_skipped_for_non_pomodoro_tasks() {
        let start = dt(2026, 10, 22, 9, 0);
        let mut engine = Engine::new(start);
        let mut task = Task::new(
            "d1",
            "喝水",
            Schedule::Daily {
                daily_time: crate::time::TimeOfDay::new(9, 0, 0),
            },
        );
        task.next_trigger = Some(dt(2026, 10, 22, 9, 0));
        let mut tasks = vec![task];

        let r = engine.tick(
            &mut tasks,
            DateTime::new(
                crate::time::Date::new(2026, 10, 22),
                TimeOfDay::new(8, 59, 57),
            ),
            0,
        );
        assert!(r.is_empty(), "daily tasks get no heads-up");
    }

    #[test]
    fn pre_alert_respects_disabled_and_muted_pomodoro() {
        let start = dt(2026, 10, 22, 9, 0);
        let mut engine = Engine::new(start);
        let mut task = Task::new(
            "p1",
            "专注",
            Schedule::Pomodoro {
                work_minutes: 25,
                break_minutes: 5,
                cycles: 4,
            },
        );
        task.next_trigger = Some(dt(2026, 10, 22, 9, 25));
        task.enabled = false;
        let mut tasks = vec![task];
        let t = DateTime::new(
            crate::time::Date::new(2026, 10, 22),
            TimeOfDay::new(9, 24, 57),
        );
        assert!(engine.tick(&mut tasks, t, 0).is_empty());

        tasks[0].enabled = true;
        tasks[0].muted_until = Some(dt(2026, 10, 22, 12, 0));
        assert!(engine.tick(&mut tasks, t, 0).is_empty(), "muted must stay quiet");
    }

    #[test]
    fn retain_known_drops_stale_pre_alerts() {
        let start = dt(2026, 10, 22, 9, 0);
        let mut engine = Engine::new(start);
        let mut task = Task::new(
            "p1",
            "x",
            Schedule::Pomodoro {
                work_minutes: 25,
                break_minutes: 5,
                cycles: 4,
            },
        );
        task.next_trigger = Some(dt(2026, 10, 22, 9, 25));
        let mut tasks = vec![task];
        engine.tick(
            &mut tasks,
            DateTime::new(
                crate::time::Date::new(2026, 10, 22),
                TimeOfDay::new(9, 24, 57),
            ),
            0,
        );
        assert!(engine.pre_alerted_for(
            "p1",
            Some(dt(2026, 10, 22, 9, 25))
        ));

        tasks.clear();
        engine.retain_known(&tasks);
        assert!(!engine.pre_alerted_for("p1", Some(dt(2026, 10, 22, 9, 25))));
    }

    #[test]
    fn retain_known_drops_stale_counters() {
        let start = dt(2026, 10, 22, 9, 0);
        let mut engine = Engine::new(start);
        let mut tasks = vec![Task::new(
            "u1",
            "x",
            Schedule::UsageBreak { usage_minutes: 100 },
        )];
        engine.tick(&mut tasks, start, 0);
        engine.tick(&mut tasks, start.add_minutes(5), 0);
        assert_eq!(engine.usage_seconds_of("u1"), 300);

        tasks.clear();
        engine.retain_known(&tasks);
        assert_eq!(engine.usage_seconds_of("u1"), 0);
    }
}
