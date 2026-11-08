//! The Settings autosave engine (`state/autosave.svelte.ts` + `autosave-core.ts`):
//! one debounced unit per autosaved domain. Pure: the owner reads each unit's
//! gate (dirty / blocked / saving) and passes explicit `Instant`s from the test
//! clock, then arms one timer for `next_deadline()`, so `advance_clock` drives it.

use std::time::{Duration, Instant};

/// Recording domains share one window (`RECORDING_AUTOSAVE_DEBOUNCE_MS`).
pub const RECORDING_DEBOUNCE: Duration = Duration::from_millis(450);
/// The source switches save at once: the tray's Start reads the saved sources.
pub const SOURCES_DEBOUNCE: Duration = Duration::ZERO;
/// The microphone controller's applies are cheaper (`MIC_AUTOSAVE_DEBOUNCE_MS`).
pub const MIC_DEBOUNCE: Duration = Duration::from_millis(250);

/// One autosave unit: the 12 recording domains, the shortcut bindings and the mic.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Unit {
    CaptureSources,
    CaptureTiming,
    Video,
    Storage,
    Display,
    Metadata,
    Inactivity,
    Processing,
    Developer,
    Access,
    AiRuntime,
    UserContext,
    Keyboard,
    Microphone,
}

impl Unit {
    pub const ALL: [Unit; 14] = [
        Unit::CaptureSources,
        Unit::CaptureTiming,
        Unit::Video,
        Unit::Storage,
        Unit::Display,
        Unit::Metadata,
        Unit::Inactivity,
        Unit::Processing,
        Unit::Developer,
        Unit::Access,
        Unit::AiRuntime,
        Unit::UserContext,
        Unit::Keyboard,
        Unit::Microphone,
    ];

    pub fn debounce(self) -> Duration {
        match self {
            Unit::CaptureSources => SOURCES_DEBOUNCE,
            Unit::Microphone => MIC_DEBOUNCE,
            _ => RECORDING_DEBOUNCE,
        }
    }

    /// The Tauri command this unit persists through.
    pub fn command(self) -> &'static str {
        match self {
            Unit::CaptureSources => "update_capture_source_settings",
            Unit::CaptureTiming => "update_capture_timing_settings",
            Unit::Video => "update_video_settings",
            Unit::Storage => "update_storage_settings",
            Unit::Display => "update_display_settings",
            Unit::Metadata => "update_metadata_settings",
            Unit::Inactivity => "update_inactivity_settings",
            Unit::Processing => "update_processing_settings",
            Unit::Developer => "update_developer_settings",
            Unit::Access => "update_access_settings",
            Unit::AiRuntime => "update_ai_runtime_settings",
            Unit::UserContext => "update_user_context_settings",
            Unit::Keyboard => "update_keyboard_bindings_settings",
            Unit::Microphone => "update_microphone_controller",
        }
    }
}

/// A unit's gate inputs, read by the owner from its drafts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Gate {
    pub dirty: bool,
    pub blocked: bool,
    pub saving: bool,
}

/// `shouldSaveDomain`: dirty, not blocked, no privacy command in flight, not saving.
pub fn ready(g: Gate, privacy_in_flight: bool) -> bool {
    g.dirty && !g.blocked && !privacy_in_flight && !g.saving
}

#[derive(Default)]
pub struct Engine {
    deadlines: Vec<(Unit, Instant)>,
}

impl Engine {
    /// Re-evaluate every unit: a unit that may save (re)arms its debounce (an
    /// edit inside the window restarts it); one that may not loses its timer.
    pub fn tick(&mut self, now: Instant, gate: impl Fn(Unit) -> Gate, privacy_in_flight: bool) {
        self.deadlines.clear();
        for unit in Unit::ALL {
            if ready(gate(unit), privacy_in_flight) {
                self.deadlines.push((unit, now + unit.debounce()));
            }
        }
    }

    /// The units whose debounce ran out by `now` and that may still save (the
    /// gate is re-checked at fire time, as `setTimeout`'s callback does).
    pub fn due(
        &mut self,
        now: Instant,
        gate: impl Fn(Unit) -> Gate,
        privacy_in_flight: bool,
    ) -> Vec<Unit> {
        let (fire, keep): (Vec<_>, Vec<_>) =
            self.deadlines.drain(..).partition(|(_, at)| *at <= now);
        self.deadlines = keep;
        fire.into_iter()
            .map(|(u, _)| u)
            .filter(|u| ready(gate(*u), privacy_in_flight))
            .collect()
    }

    /// Save one unit now (Start flushes the sources): drops its timer.
    pub fn take(&mut self, unit: Unit) {
        self.deadlines.retain(|(u, _)| *u != unit);
    }

    pub fn cancel_all(&mut self) {
        self.deadlines.clear();
    }

    pub fn next_deadline(&self) -> Option<Instant> {
        self.deadlines.iter().map(|(_, at)| *at).min()
    }

    pub fn pending(&self, unit: Unit) -> bool {
        self.deadlines.iter().any(|(u, _)| *u == unit)
    }
}

/// The bar's one save status (`deriveSaveStatus`): failed > blocked > saving > saved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveStatus {
    Failed,
    Blocked,
    Saving,
    Saved,
}

pub fn save_status(failed: bool, blocked: bool, saving: bool) -> SaveStatus {
    if failed {
        SaveStatus::Failed
    } else if blocked {
        SaveStatus::Blocked
    } else if saving {
        SaveStatus::Saving
    } else {
        SaveStatus::Saved
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    fn only(unit: Unit) -> impl Fn(Unit) -> Gate {
        move |u| Gate {
            dirty: u == unit,
            ..Gate::default()
        }
    }

    #[test]
    fn each_window_fires_on_its_boundary() {
        let t0 = Instant::now();
        for (unit, ms) in [
            (Unit::Display, 450),
            (Unit::CaptureSources, 0),
            (Unit::Microphone, 250),
        ] {
            let mut e = Engine::default();
            e.tick(t0, only(unit), false);
            if ms > 0 {
                assert!(
                    e.due(t0 + (ms - 1) * MS, only(unit), false).is_empty(),
                    "{unit:?} early"
                );
            }
            assert_eq!(e.due(t0 + ms * MS, only(unit), false), vec![unit]);
        }
    }

    #[test]
    fn an_edit_restarts_the_window() {
        let t0 = Instant::now();
        let mut e = Engine::default();
        e.tick(t0, only(Unit::Video), false);
        e.tick(t0 + 300 * MS, only(Unit::Video), false);
        assert!(e.due(t0 + 450 * MS, only(Unit::Video), false).is_empty());
        assert_eq!(
            e.due(t0 + 750 * MS, only(Unit::Video), false),
            vec![Unit::Video]
        );
    }

    #[test]
    fn privacy_command_blocks_every_unit() {
        let t0 = Instant::now();
        let mut e = Engine::default();
        e.tick(t0, only(Unit::CaptureSources), true);
        assert_eq!(e.next_deadline(), None);
        // Armed before the command started, re-checked at fire time.
        e.tick(t0, only(Unit::CaptureSources), false);
        assert!(e.due(t0, only(Unit::CaptureSources), true).is_empty());
    }

    #[test]
    fn blocked_saving_and_clean_units_never_arm() {
        let t0 = Instant::now();
        let mut e = Engine::default();
        let g = |u| Gate {
            dirty: u != Unit::Display,
            blocked: u == Unit::Video,
            saving: u == Unit::Storage,
        };
        e.tick(t0, g, false);
        assert!(!e.pending(Unit::Display));
        assert!(!e.pending(Unit::Video));
        assert!(!e.pending(Unit::Storage));
        assert!(e.pending(Unit::Access));
    }

    #[test]
    fn status_priority() {
        assert_eq!(save_status(true, true, true), SaveStatus::Failed);
        assert_eq!(save_status(false, true, true), SaveStatus::Blocked);
        assert_eq!(save_status(false, false, true), SaveStatus::Saving);
        assert_eq!(save_status(false, false, false), SaveStatus::Saved);
    }
}
