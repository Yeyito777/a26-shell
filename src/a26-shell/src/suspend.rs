use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SuspendPhase {
    Awake,
    PreparingSleep,
    ScreenOff,
    Waking,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuspendAction {
    None,
    Sleep,
    Wake,
}

#[derive(Debug, Clone, Serialize)]
pub struct PublicSuspendState {
    pub phase: SuspendPhase,
    pub hardware_awake: bool,
    pub last_error: Option<&'static str>,
    pub transitions: u64,
}

#[derive(Debug)]
pub struct SuspendCoordinator {
    phase: SuspendPhase,
    hardware_awake: bool,
    last_error: Option<&'static str>,
    transitions: u64,
}

impl SuspendCoordinator {
    pub fn new(hardware_awake: bool) -> Self {
        Self {
            phase: if hardware_awake {
                SuspendPhase::Awake
            } else {
                SuspendPhase::ScreenOff
            },
            hardware_awake,
            last_error: None,
            transitions: 0,
        }
    }

    pub fn next_action(&mut self, requested_awake: bool) -> SuspendAction {
        if requested_awake == self.hardware_awake {
            return SuspendAction::None;
        }
        self.phase = if requested_awake {
            SuspendPhase::Waking
        } else {
            SuspendPhase::PreparingSleep
        };
        if requested_awake {
            SuspendAction::Wake
        } else {
            SuspendAction::Sleep
        }
    }

    pub fn complete_sleep(&mut self) {
        self.hardware_awake = false;
        self.phase = SuspendPhase::ScreenOff;
        self.last_error = None;
        self.transitions = self.transitions.saturating_add(1);
    }

    pub fn complete_wake(&mut self) {
        self.hardware_awake = true;
        self.phase = SuspendPhase::Awake;
        self.last_error = None;
        self.transitions = self.transitions.saturating_add(1);
    }

    pub fn fail_sleep(&mut self, code: &'static str) {
        // The caller performs best-effort panel/touch rollback before reporting
        // failure. Treat the hardware as awake so the failed request cannot
        // become a retry loop that repeatedly blanks the panel.
        self.hardware_awake = true;
        self.phase = SuspendPhase::Awake;
        self.last_error = Some(code);
    }

    pub fn fail_wake(&mut self, code: &'static str) {
        // A partial wake is safer to model as awake and locked. The next explicit
        // power transition can perform a complete off/on sequence.
        self.hardware_awake = true;
        self.phase = SuspendPhase::Awake;
        self.last_error = Some(code);
    }

    pub fn public(&self) -> PublicSuspendState {
        PublicSuspendState {
            phase: self.phase,
            hardware_awake: self.hardware_awake,
            last_error: self.last_error,
            transitions: self.transitions,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coordinator_has_explicit_sleep_and_wake_commits() {
        let mut coordinator = SuspendCoordinator::new(true);
        assert_eq!(coordinator.next_action(false), SuspendAction::Sleep);
        assert_eq!(coordinator.public().phase, SuspendPhase::PreparingSleep);
        coordinator.complete_sleep();
        assert_eq!(coordinator.public().phase, SuspendPhase::ScreenOff);
        assert!(!coordinator.public().hardware_awake);
        assert_eq!(coordinator.next_action(true), SuspendAction::Wake);
        assert_eq!(coordinator.public().phase, SuspendPhase::Waking);
        coordinator.complete_wake();
        assert_eq!(coordinator.public().phase, SuspendPhase::Awake);
        assert_eq!(coordinator.public().transitions, 2);
    }

    #[test]
    fn failed_sleep_rolls_back_to_an_awake_stable_state() {
        let mut coordinator = SuspendCoordinator::new(true);
        assert_eq!(coordinator.next_action(false), SuspendAction::Sleep);
        coordinator.fail_sleep("touchscreen_sleep_failed");
        let state = coordinator.public();
        assert_eq!(state.phase, SuspendPhase::Awake);
        assert!(state.hardware_awake);
        assert_eq!(state.last_error, Some("touchscreen_sleep_failed"));
    }
}
