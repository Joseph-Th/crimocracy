//! Player time advancement, with interruption and terminal-clock semantics kept in one place.

use super::*;

/// Clamp requested one-minute ticks to the finite simulation clock before mutation. Without
/// this preflight a multi-minute command could advance partway, then return a rejection when
/// the next minute was no longer representable.
fn bounded_advance_span(
    now: crimocracy::core::time::SimTime,
    requested: u64,
) -> Result<u64, String> {
    let available = u64::MAX - now.as_minutes();
    if available == 0 {
        return Err("simulation clock is exhausted; no further advance is possible".to_owned());
    }
    Ok(requested.min(available))
}

impl Session<'_> {
    pub(super) fn execute_advance(&mut self, duration: &str) -> Result<String, String> {
        let requested = parse_advance(duration)?;
        if self
            .state
            .decisions()
            .pending_for_recipient(self.player)
            .next()
            .is_some()
        {
            return Err("resolve your pending decisions before advancing".to_owned());
        }
        let span = bounded_advance_span(self.state.now(), requested)?;
        let mut advanced = 0;
        let mut new_reports = 0;
        let mut reason = None;
        for _ in 0..span {
            let previous = self
                .state
                .reports()
                .reports_for(self.player)
                .next_back()
                .map(|r| r.id());
            let tick = run_tick(self.registry, &mut self.state).map_err(|e| e.to_string())?;
            advanced += 1;
            let reports: Vec<_> = self
                .state
                .reports()
                .reports_for_after(self.player, previous)
                .collect();
            new_reports += reports.len();
            let urgent = reports.iter().any(|report| {
                report.entries().iter().any(|entry| {
                    entry.attention.can_auto_pause()
                        && self
                            .state
                            .attention_settings()
                            .is_auto_pause_enabled(entry.attention)
                })
            });
            let operation_ended = tick
                .resolved_operations
                .iter()
                .chain(&tick.aborted_operations)
                .any(|id| {
                    self.state
                        .operations()
                        .get_operation(*id)
                        .is_some_and(|op| op.responsible_organization() == self.player)
                });
            let own_custody_event = |arrests: &[ArrestId]| {
                arrests.iter().any(|id| {
                    self.state.legal().get_arrest(*id).is_some_and(|arrest| {
                        self.state
                            .world()
                            .get_character(arrest.character())
                            .is_some_and(|person| person.organization() == Some(self.player))
                    })
                })
            };
            let own_arrest = own_custody_event(&tick.evidence_arrests);
            let own_release = own_custody_event(&tick.custody_releases);
            if self
                .state
                .decisions()
                .pending_for_recipient(self.player)
                .next()
                .is_some()
            {
                reason = Some("leadership decision pending");
            } else if own_arrest {
                reason = Some("your member entered custody; review 'counsel' and 'contacts'");
            } else if own_release {
                reason = Some("your member was released from custody; review 'counsel'");
            } else if urgent {
                reason = Some("high-priority report");
            } else if operation_ended {
                reason = Some("your operation reached a terminal outcome");
            }
            if reason.is_some() {
                break;
            }
        }
        let mut out = String::new();
        writeln!(
            out,
            "Advanced {advanced} minute(s) to {}. {new_reports} new report(s).",
            time_label(self.state.now().as_minutes())
        )
        .unwrap();
        if let Some(reason) = reason {
            writeln!(out, "Paused: {reason}.").unwrap();
        }
        if self.state.now() == crimocracy::core::time::SimTime::MAX {
            writeln!(
                out,
                "Simulation clock limit reached; no further minutes can advance."
            )
            .unwrap();
        }
        for report in self
            .state
            .reports()
            .reports_for(self.player)
            .rev()
            .take(new_reports.min(6))
        {
            writeln!(out, "  report {}: {}", report.id().raw(), report.title()).unwrap();
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crimocracy::core::time::SimTime;

    #[test]
    fn advancement_clamps_to_the_final_representable_minute() {
        assert_eq!(bounded_advance_span(SimTime::ZERO, 20).unwrap(), 20);
        assert_eq!(
            bounded_advance_span(SimTime::from_minutes(u64::MAX - 2), 20).unwrap(),
            2
        );
        assert_eq!(
            bounded_advance_span(SimTime::from_minutes(u64::MAX - 5), 2).unwrap(),
            2
        );
        assert!(bounded_advance_span(SimTime::MAX, 1).is_err());
    }
}
