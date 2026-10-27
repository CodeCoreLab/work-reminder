// Copyright (c) 2026 Ma Xin <maxin.zhang@users.noreply.github.com>
// SPDX-License-Identifier: MIT
//
// Schedule arithmetic: when does a task fire next, and what state follows a
// firing.
//
// This is a clean-room reimplementation of the behaviour documented in
// docs/reverse-analysis.md §3.2 (next_trigger, original routine 0x17160) and
// §3.3 (pomodoro state machine, original routine 0x160f0). Behaviour that the
// reference implementation got right is preserved; the previously shared
// mutable state fields are now an explicit enum plus a dedicated state struct.

use crate::model::{MonthlyMode, Schedule, Task, WORKDAY_MASK};
use crate::time::{Date, DateTime, TimeOfDay};

/// Returned by [`Scheduler::is_due`] when a task was consumed by this check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FireOutcome {
    /// The task fired; `next_trigger` was advanced.
    Fired,
    /// The task fired and has no further occurrence (finished or expired).
    FiredAndFinished,
    /// Not due yet.
    NotDue,
}

/// Compute the next firing instant for `schedule`, given `now` and the point at
/// which the schedule was armed (`anchor`, used by startup / interval kinds).
pub fn next_trigger(schedule: &Schedule, now: DateTime, anchor: DateTime) -> Option<DateTime> {
    match schedule {
        Schedule::Interval { interval_minutes } => {
            Some(now.add_minutes((*interval_minutes).max(1)))
        }
        Schedule::Daily { daily_time } => Some(next_daily(*daily_time, now)),
        Schedule::Once { once_at } => {
            if *once_at > now {
                Some(*once_at)
            } else {
                None
            }
        }
        Schedule::Weekly {
            weekdays_mask,
            daily_time,
        } => next_masked_day(*weekdays_mask, *daily_time, now),
        Schedule::Workdays { daily_time } => next_masked_day(WORKDAY_MASK, *daily_time, now),
        Schedule::Monthly {
            monthly_mode,
            monthly_day,
            daily_time,
        } => Some(next_monthly(*monthly_mode, *monthly_day, *daily_time, now)),
        Schedule::MultiDaily { daily_times } => next_multi_daily(daily_times, now),
        Schedule::TimedInterval {
            weekdays_mask,
            active_start,
            active_end,
            interval_minutes,
        } => next_timed_interval(
            *weekdays_mask,
            *active_start,
            *active_end,
            (*interval_minutes).max(1),
            now,
        ),
        Schedule::Countdown { countdown_minutes } => {
            Some(now.add_minutes((*countdown_minutes).max(1)))
        }
        Schedule::Pomodoro { work_minutes, .. } => {
            Some(now.add_minutes((*work_minutes).max(1)))
        }
        Schedule::AfterStartup {
            startup_delay_minutes,
        } => Some(anchor.add_minutes((*startup_delay_minutes).max(1))),
        Schedule::UsageBreak { usage_minutes } => Some(now.add_minutes((*usage_minutes).max(1))),
    }
}

/// Today's occurrence if still ahead of `now`, otherwise tomorrow's.
fn next_daily(at: TimeOfDay, now: DateTime) -> DateTime {
    let today = DateTime::new(now.date, at);
    if today > now {
        today
    } else {
        DateTime::new(now.date.next_day(), at)
    }
}

/// Scan forward up to two weeks for the first masked weekday whose time has not
/// passed. Two weeks bounds the search because every non-empty mask repeats
/// within seven days.
fn next_masked_day(mask: u8, at: TimeOfDay, now: DateTime) -> Option<DateTime> {
    if mask == 0 {
        return None;
    }
    let mut date = now.date;
    for _ in 0..14 {
        let weekday = date.weekday(); // 1..7, Monday = 1
        if mask & (1 << (weekday - 1)) != 0 {
            let candidate = DateTime::new(date, at);
            if candidate > now {
                return Some(candidate);
            }
        }
        date = date.next_day();
    }
    None
}

fn next_monthly(mode: MonthlyMode, day: u32, at: TimeOfDay, now: DateTime) -> DateTime {
    // Try this month first, then advance month by month until an occurrence is
    // strictly in the future. Twelve attempts always suffice.
    let mut year = now.date.year;
    let mut month = now.date.month;
    for _ in 0..12 {
        if let Some(date) = monthly_anchor(year, month, mode, day) {
            let candidate = DateTime::new(date, at);
            if candidate > now {
                return candidate;
            }
        }
        if month == 12 {
            month = 1;
            year += 1;
        } else {
            month += 1;
        }
    }
    // Unreachable for valid input; fall back to "today plus a month".
    DateTime::new(now.date.next_day(), at)
}

fn monthly_anchor(year: i32, month: u32, mode: MonthlyMode, day: u32) -> Option<Date> {
    match mode {
        MonthlyMode::Day => Some(Date::new(year, month, day).with_day(day)),
        MonthlyMode::LastDay => {
            let dim = Date::days_in_month(year, month);
            if dim == 0 {
                None
            } else {
                Some(Date::new(year, month, dim))
            }
        }
        MonthlyMode::LastWorkday => {
            let dim = Date::days_in_month(year, month);
            if dim == 0 {
                return None;
            }
            let mut date = Date::new(year, month, dim);
            // Walk back over weekends only, so the anchor stays in this month.
            while date.is_weekend() && date.day > 1 {
                date = date.prev_day();
            }
            Some(date)
        }
    }
}

fn next_multi_daily(times: &[TimeOfDay], now: DateTime) -> Option<DateTime> {
    if times.is_empty() {
        return None;
    }
    let mut sorted: Vec<TimeOfDay> = times.to_vec();
    sorted.sort();
    for t in &sorted {
        let candidate = DateTime::new(now.date, *t);
        if candidate > now {
            return Some(candidate);
        }
    }
    Some(DateTime::new(now.date.next_day(), sorted[0]))
}

/// Inside the window: align to the next multiple of `step` measured from
/// `active_start`. Outside the window: jump to the next window opening.
fn next_timed_interval(
    mask: u8,
    start: TimeOfDay,
    end: TimeOfDay,
    step: i64,
    now: DateTime,
) -> Option<DateTime> {
    if mask == 0 || end <= start {
        return None;
    }
    let mut date = now.date;
    for _ in 0..14 {
        let weekday = date.weekday();
        if mask & (1 << (weekday - 1)) != 0 {
            let window_start = DateTime::new(date, start);
            let window_end = DateTime::new(date, end);
            if now < window_start {
                // First slot of the window.
                let candidate = window_start.add_minutes(step);
                if candidate <= window_end {
                    return Some(candidate);
                }
            } else if now < window_end {
                let elapsed = window_start.minutes_until(&now);
                let slots = elapsed / step + 1;
                let candidate = window_start.add_minutes(slots * step);
                if candidate <= window_end {
                    return Some(candidate);
                }
            }
        }
        date = date.next_day();
    }
    None
}

/// Whether a task should fire at `now`, and if so advance its state.
///
/// `anchor` is the instant the schedule was armed; `usage_seconds` is the
/// accumulated continuous-usage counter maintained by the tick loop, which only
/// applies to `UsageBreak` tasks.
pub fn is_due(task: &mut Task, now: DateTime, anchor: DateTime, usage_seconds: i64) -> FireOutcome {
    if !task.is_active_at(&now) {
        return FireOutcome::NotDue;
    }

    // A usage-break task is not time-scheduled; it fires on accumulated usage.
    if let Schedule::UsageBreak { usage_minutes } = task.schedule {
        if usage_seconds < usage_minutes.max(1) * 60 {
            return FireOutcome::NotDue;
        }
        return FireOutcome::Fired;
    }

    match task.next_trigger {
        None => {
            // First evaluation after creation / load: arm and wait.
            task.next_trigger = next_trigger(&task.schedule, now, anchor);
            FireOutcome::NotDue
        }
        Some(due) if now >= due => {
            
            advance(task, now, anchor)
        }
        Some(_) => FireOutcome::NotDue,
    }
}

/// Post-fire state transition. Pomodoro alternates work and break; everything
/// else simply recomputes the next occurrence and finishes if there is none.
fn advance(task: &mut Task, now: DateTime, anchor: DateTime) -> FireOutcome {
    if let Schedule::Pomodoro {
        work_minutes,
        break_minutes,
        cycles,
    } = task.schedule
    {
        // Work phase just finished: the next phase is a break, so the delay is
        // the break length. Mirrors 0x16253-0x16267, which multiplies
        // breakMinutes (+0x48) by 60.
        if !task.pomodoro.in_break {
            task.pomodoro.in_break = true;
            task.next_trigger = Some(now.add_minutes(break_minutes.max(1)));
            return FireOutcome::Fired;
        }
        // Break finished: count the completed work session and either start the
        // next work phase or stop once every cycle has run. Mirrors
        // 0x1628b-0x162b4, which increments the cycle counter and reads
        // workMinutes (+0x44).
        task.pomodoro.in_break = false;
        task.pomodoro.current_cycle += 1;
        if task.pomodoro.current_cycle >= cycles.max(1) {
            task.enabled = false;
            task.next_trigger = None;
            return FireOutcome::FiredAndFinished;
        }
        task.next_trigger = Some(now.add_minutes(work_minutes.max(1)));
        return FireOutcome::Fired;
    }

    // One-shot kinds (a single absolute datetime, or a countdown) have nothing
    // to re-arm after firing, so they end here. Recurring kinds recompute.
    let single_shot = matches!(
        task.schedule,
        Schedule::Once { .. } | Schedule::Countdown { .. }
    );
    if single_shot {
        task.enabled = false;
        task.next_trigger = None;
        return FireOutcome::FiredAndFinished;
    }

    match next_trigger(&task.schedule, now, anchor) {
        Some(next) => {
            task.next_trigger = Some(next);
            FireOutcome::Fired
        }
        None => {
            // No further occurrence (for example an expired masked schedule).
            task.enabled = false;
            task.next_trigger = None;
            FireOutcome::FiredAndFinished
        }
    }
}

/// Reset a pomodoro task back to its first work phase.
pub fn restart_pomodoro(task: &mut Task, now: DateTime) {
    if let Schedule::Pomodoro { work_minutes, .. } = task.schedule {
        task.pomodoro = crate::model::PomodoroState::default();
        task.enabled = true;
        task.next_trigger = Some(now.add_minutes(work_minutes.max(1)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{PomodoroState, Task};

    fn dt(y: i32, m: u32, d: u32, hh: u32, mm: u32) -> DateTime {
        DateTime::new(Date::new(y, m, d), TimeOfDay::new(hh, mm, 0))
    }

    #[test]
    fn daily_rolls_to_tomorrow_when_time_passed() {
        let s = Schedule::Daily {
            daily_time: TimeOfDay::new(9, 0, 0),
        };
        let now = dt(2026, 10, 22, 8, 0);
        assert_eq!(
            next_trigger(&s, now, now),
            Some(dt(2026, 10, 22, 9, 0))
        );

        let now = dt(2026, 10, 22, 10, 0);
        assert_eq!(
            next_trigger(&s, now, now),
            Some(dt(2026, 10, 23, 9, 0))
        );
    }

    #[test]
    fn once_in_the_past_has_no_trigger() {
        let s = Schedule::Once {
            once_at: dt(2026, 10, 20, 9, 0),
        };
        let now = dt(2026, 10, 22, 10, 0);
        assert_eq!(next_trigger(&s, now, now), None);

        let future = Schedule::Once {
            once_at: dt(2026, 10, 25, 9, 0),
        };
        assert_eq!(
            next_trigger(&future, now, now),
            Some(dt(2026, 10, 25, 9, 0))
        );
    }

    #[test]
    fn weekly_picks_first_masked_day() {
        // 2026-10-22 is Thursday (bit3). Mask = Monday + Friday.
        let s = Schedule::Weekly {
            weekdays_mask: 0b001_0001,
            daily_time: TimeOfDay::new(9, 0, 0),
        };
        let now = dt(2026, 10, 22, 10, 0);
        // Friday 2026-10-23 comes before Monday 2026-10-26.
        assert_eq!(
            next_trigger(&s, now, now),
            Some(dt(2026, 10, 23, 9, 0))
        );
    }

    #[test]
    fn workdays_skips_weekend() {
        let s = Schedule::Workdays {
            daily_time: TimeOfDay::new(9, 0, 0),
        };
        // Friday 2026-10-23 after 09:00 → next is Monday 2026-10-26.
        let now = dt(2026, 10, 23, 10, 0);
        assert_eq!(
            next_trigger(&s, now, now),
            Some(dt(2026, 10, 26, 9, 0))
        );
    }

    #[test]
    fn weekly_with_empty_mask_never_fires() {
        let s = Schedule::Weekly {
            weekdays_mask: 0,
            daily_time: TimeOfDay::new(9, 0, 0),
        };
        let now = dt(2026, 10, 22, 10, 0);
        assert_eq!(next_trigger(&s, now, now), None);
    }

    #[test]
    fn monthly_day_clamps_to_short_months() {
        let s = Schedule::Monthly {
            monthly_mode: MonthlyMode::Day,
            monthly_day: 31,
            daily_time: TimeOfDay::new(9, 0, 0),
        };
        // November has 30 days; asking for the 31st must land on the 30th.
        let now = dt(2026, 11, 15, 10, 0);
        assert_eq!(
            next_trigger(&s, now, now),
            Some(dt(2026, 11, 30, 9, 0))
        );
    }

    #[test]
    fn monthly_last_workday_skips_weekend() {
        let s = Schedule::Monthly {
            monthly_mode: MonthlyMode::LastWorkday,
            monthly_day: 1,
            daily_time: TimeOfDay::new(9, 0, 0),
        };
        // 2026-10-31 is a Saturday → last workday is Friday the 30th.
        let now = dt(2026, 10, 1, 10, 0);
        assert_eq!(
            next_trigger(&s, now, now),
            Some(dt(2026, 10, 30, 9, 0))
        );
    }

    #[test]
    fn monthly_last_day_is_calendar_end() {
        let s = Schedule::Monthly {
            monthly_mode: MonthlyMode::LastDay,
            monthly_day: 1,
            daily_time: TimeOfDay::new(9, 0, 0),
        };
        let now = dt(2026, 2, 1, 10, 0);
        assert_eq!(
            next_trigger(&s, now, now),
            Some(dt(2026, 2, 28, 9, 0))
        );
    }

    #[test]
    fn multi_daily_wraps_to_next_day() {
        let s = Schedule::MultiDaily {
            daily_times: vec![
                TimeOfDay::new(17, 30, 0),
                TimeOfDay::new(9, 0, 0),
                TimeOfDay::new(12, 0, 0),
            ],
        };
        let now = dt(2026, 10, 22, 13, 0);
        assert_eq!(
            next_trigger(&s, now, now),
            Some(dt(2026, 10, 22, 17, 30))
        );

        let late = dt(2026, 10, 22, 20, 0);
        assert_eq!(
            next_trigger(&s, late, late),
            Some(dt(2026, 10, 23, 9, 0))
        );
    }

    #[test]
    fn timed_interval_aligns_to_window_grid() {
        let s = Schedule::TimedInterval {
            weekdays_mask: 0b001_1111,
            active_start: TimeOfDay::new(9, 0, 0),
            active_end: TimeOfDay::new(18, 0, 0),
            interval_minutes: 60,
        };
        // Thursday 10:15 → next slot is 11:00 (grid from 09:00, step 60).
        let now = dt(2026, 10, 22, 10, 15);
        assert_eq!(
            next_trigger(&s, now, now),
            Some(dt(2026, 10, 22, 11, 0))
        );

        // Thursday 08:00 (before the window) → first slot at 10:00.
        let early = dt(2026, 10, 22, 8, 0);
        assert_eq!(
            next_trigger(&s, early, early),
            Some(dt(2026, 10, 22, 10, 0))
        );
    }

    #[test]
    fn timed_interval_outside_window_moves_to_next_workday() {
        let s = Schedule::TimedInterval {
            weekdays_mask: 0b001_1111,
            active_start: TimeOfDay::new(9, 0, 0),
            active_end: TimeOfDay::new(18, 0, 0),
            interval_minutes: 60,
        };
        // Friday 19:00 → next window Monday 10:00.
        let now = dt(2026, 10, 23, 19, 0);
        assert_eq!(
            next_trigger(&s, now, now),
            Some(dt(2026, 10, 26, 10, 0))
        );
    }

    #[test]
    fn timed_interval_rejects_inverted_window() {
        let s = Schedule::TimedInterval {
            weekdays_mask: 0b001_1111,
            active_start: TimeOfDay::new(18, 0, 0),
            active_end: TimeOfDay::new(9, 0, 0),
            interval_minutes: 60,
        };
        let now = dt(2026, 10, 22, 10, 0);
        assert_eq!(next_trigger(&s, now, now), None);
    }

    #[test]
    fn after_startup_measures_from_anchor_not_now() {
        let s = Schedule::AfterStartup {
            startup_delay_minutes: 5,
        };
        let anchor = dt(2026, 10, 22, 8, 0);
        let now = dt(2026, 10, 22, 12, 0);
        assert_eq!(
            next_trigger(&s, now, anchor),
            Some(dt(2026, 10, 22, 8, 5))
        );
    }

    #[test]
    fn pomodoro_alternates_work_and_break_then_finishes() {
        // Cycles = 2 means two work sessions. Timeline with work=25, break=5:
        //   09:00 arm work          -> fires 09:25
        //   09:25 work done        -> break until 09:30
        //   09:30 break done       -> cycle 1, work until 09:55
        //   09:55 work done        -> break until 10:00
        //   10:00 break done       -> cycle 2 == cycles, task finishes
        let mut task = Task::new(
            "p",
            "专注",
            Schedule::Pomodoro {
                work_minutes: 25,
                break_minutes: 5,
                cycles: 2,
            },
        );
        let t0 = dt(2026, 10, 22, 9, 0);
        task.next_trigger = Some(dt(2026, 10, 22, 9, 25));

        // 09:25 - work finished, enter break.
        let t = dt(2026, 10, 22, 9, 25);
        assert_eq!(is_due(&mut task, t, t0, 0), FireOutcome::Fired);
        assert!(task.pomodoro.in_break);
        assert_eq!(task.pomodoro.current_cycle, 0);
        assert_eq!(task.next_trigger, Some(dt(2026, 10, 22, 9, 30)));

        // 09:30 - break finished, cycle 1, next work phase.
        let t = dt(2026, 10, 22, 9, 30);
        assert_eq!(is_due(&mut task, t, t0, 0), FireOutcome::Fired);
        assert!(!task.pomodoro.in_break);
        assert_eq!(task.pomodoro.current_cycle, 1);
        assert_eq!(task.next_trigger, Some(dt(2026, 10, 22, 9, 55)));

        // 09:55 - work finished, enter the second break.
        let t = dt(2026, 10, 22, 9, 55);
        assert_eq!(is_due(&mut task, t, t0, 0), FireOutcome::Fired);
        assert!(task.pomodoro.in_break);
        assert_eq!(task.next_trigger, Some(dt(2026, 10, 22, 10, 0)));

        // 10:00 - last break finished, cycles exhausted.
        let t = dt(2026, 10, 22, 10, 0);
        assert_eq!(
            is_due(&mut task, t, t0, 0),
            FireOutcome::FiredAndFinished
        );
        assert!(!task.enabled);
        assert_eq!(task.next_trigger, None);

        // A finished pomodoro stays silent until re-armed.
        let t = dt(2026, 10, 22, 11, 0);
        assert_eq!(is_due(&mut task, t, t0, 0), FireOutcome::NotDue);
    }

    #[test]
    fn restart_resets_pomodoro_state() {
        let mut task = Task::new(
            "p",
            "专注",
            Schedule::Pomodoro {
                work_minutes: 25,
                break_minutes: 5,
                cycles: 4,
            },
        );
        task.pomodoro = PomodoroState {
            current_cycle: 3,
            in_break: true,
        };
        task.enabled = false;
        let now = dt(2026, 10, 22, 14, 0);
        restart_pomodoro(&mut task, now);
        assert_eq!(task.pomodoro, PomodoroState::default());
        assert!(task.enabled);
        assert_eq!(task.next_trigger, Some(dt(2026, 10, 22, 14, 25)));
    }

    #[test]
    fn usage_break_fires_on_accumulated_seconds() {
        let mut task = Task::new(
            "u",
            "起来活动",
            Schedule::UsageBreak { usage_minutes: 45 },
        );
        let now = dt(2026, 10, 22, 10, 0);

        assert_eq!(is_due(&mut task, now, now, 44 * 60), FireOutcome::NotDue);
        assert_eq!(is_due(&mut task, now, now, 45 * 60), FireOutcome::Fired);
    }

    #[test]
    fn muted_task_does_not_fire() {
        let mut task = Task::new("m", "x", Schedule::Interval { interval_minutes: 1 });
        let now = dt(2026, 10, 22, 10, 0);
        task.next_trigger = Some(now);
        task.muted_until = Some(dt(2026, 10, 22, 11, 0));
        assert_eq!(is_due(&mut task, now, now, 0), FireOutcome::NotDue);
    }

    #[test]
    fn first_evaluation_arms_without_firing() {
        let mut task = Task::new("a", "x", Schedule::Daily {
            daily_time: TimeOfDay::new(9, 0, 0),
        });
        let now = dt(2026, 10, 22, 8, 0);
        assert_eq!(is_due(&mut task, now, now, 0), FireOutcome::NotDue);
        assert_eq!(task.next_trigger, Some(dt(2026, 10, 22, 9, 0)));
    }

    #[test]
    fn countdown_finishes_after_single_fire() {
        let mut task = Task::new("c", "x", Schedule::Countdown {
            countdown_minutes: 10,
        });
        let now = dt(2026, 10, 22, 10, 0);
        task.next_trigger = Some(now);
        assert_eq!(
            is_due(&mut task, now, now, 0),
            FireOutcome::FiredAndFinished
        );
        assert!(!task.enabled);
    }
}
