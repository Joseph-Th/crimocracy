//! Exact integration of recurring patrol windows over arbitrary simulation intervals.
//!
//! Patrol administration and historical revisions live in the parent module; this pure
//! projection owns the end-exclusive daily boundaries and overflow-safe elapsed averages.

use crate::core::time::{DAY_MINUTES_U16, SimTime};
use crate::legal::PatrolWindow;
use std::collections::BTreeMap;

/// End-exclusive minute boundaries and maximum active presence until each boundary. A window
/// crossing midnight is split in two; concurrent precincts contribute their maximum presence,
/// never their sum. Uncovered minutes use reduced ambient presence.
pub(super) fn patrol_daily_presence_segments(
    windows: &[PatrolWindow],
    off_window_presence: u8,
) -> Vec<(u16, u8)> {
    let mut events: BTreeMap<u16, Vec<(u8, bool)>> = BTreeMap::new();
    events.entry(0).or_default();
    events.entry(DAY_MINUTES_U16).or_default();
    for window in windows {
        let start = u32::from(window.start().value());
        let end = start + u32::from(window.duration_minutes());
        let day = u32::from(DAY_MINUTES_U16);
        let presence = window.presence().value();
        let mut record_interval = |start: u16, end: u16| {
            events.entry(start).or_default().push((presence, true));
            events.entry(end).or_default().push((presence, false));
        };
        record_interval(start as u16, end.min(day) as u16);
        if end > day {
            record_interval(0, (end - day) as u16);
        }
    }

    let mut active: BTreeMap<u8, u32> = BTreeMap::new();
    let mut segments = Vec::with_capacity(events.len());
    let mut previous = 0;
    for (minute, changes) in events {
        if minute > previous {
            let presence = active
                .last_key_value()
                .map(|(presence, _)| *presence)
                .unwrap_or(off_window_presence);
            segments.push((minute, presence));
            previous = minute;
        }
        for (presence, entering) in changes {
            if entering {
                *active.entry(presence).or_default() += 1;
            } else {
                let count = active
                    .get_mut(&presence)
                    .expect("each patrol window exit must match its entry");
                *count -= 1;
                if *count == 0 {
                    active.remove(&presence);
                }
            }
        }
    }
    debug_assert!(active.is_empty());
    debug_assert_eq!(
        segments.last().map(|(minute, _)| *minute),
        Some(DAY_MINUTES_U16)
    );
    segments
}

pub(super) fn presence_sum_between(segments: &[(u16, u8)], start: u16, end: u16) -> u128 {
    let mut previous = 0;
    let mut total = 0;
    for &(boundary, presence) in segments {
        if previous >= end {
            break;
        }
        let from = previous.max(start);
        let until = boundary.min(end);
        if until > from {
            total += u128::from(until - from) * u128::from(presence);
        }
        previous = boundary;
    }
    total
}

pub(super) fn scheduled_presence_sum(
    segments: &[(u16, u8)],
    start: SimTime,
    duration: u64,
) -> u128 {
    let day_minutes = u64::from(DAY_MINUTES_U16);
    // Per-minute presence is bounded by 100, so u128 preserves the exact total across the
    // entire u64 clock horizon without ever iterating over elapsed campaign days.
    let daily_total = presence_sum_between(segments, 0, DAY_MINUTES_U16);
    let full_days = duration / day_minutes;
    let remainder = duration % day_minutes;
    let mut total_presence = daily_total * u128::from(full_days);
    let start_minute = start.as_minutes() % day_minutes;
    let end_minute = start_minute + remainder;
    total_presence += presence_sum_between(
        segments,
        start_minute as u16,
        end_minute.min(day_minutes) as u16,
    );
    if end_minute > day_minutes {
        total_presence += presence_sum_between(segments, 0, (end_minute - day_minutes) as u16);
    }
    total_presence
}
