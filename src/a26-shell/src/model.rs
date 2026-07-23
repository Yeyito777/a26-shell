use serde::Serialize;
use sha2::{Digest, Sha256};
use std::time::{Duration, Instant};
use subtle::ConstantTimeEq;

use crate::config::Config;
use crate::keyboard::{
    KeyAction, KeyboardEffect, KeyboardPurpose, KeyboardState, PublicKeyboardState,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum View {
    Locked,
    Launcher,
    System,
    Browser,
}

impl View {
    pub fn is_app(self) -> bool {
        matches!(self, Self::System | Self::Browser)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PointerGesture {
    pub start_x: i16,
    pub start_y: i16,
    pub last_x: i16,
    pub last_y: i16,
    pub started: Instant,
    pub keyboard_owned: bool,
    pub keyboard_key_index: Option<usize>,
}

#[derive(Debug)]
pub struct ShellState {
    pub view: View,
    pub screen_awake: bool,
    pub pin_input: Vec<u8>,
    pub failed_attempts: u32,
    pub lockout_until: Option<Instant>,
    pub volume: u8,
    pub volume_overlay_until: Option<Instant>,
    pub battery_percent: Option<u8>,
    pub wifi_connected: bool,
    pub pointer: Option<PointerGesture>,
    pub last_action: String,
    pub redraw: bool,
    pub should_exit: bool,
    pub managed_windows: Vec<u32>,
    pub keyboard: KeyboardState,
    app_launch_started: Option<Instant>,
    app_window_mapped_at: Option<Instant>,
}

#[derive(Debug, Serialize)]
pub struct PublicState {
    pub version: &'static str,
    pub view: View,
    pub locked: bool,
    pub screen_awake: bool,
    pub pin_digits: usize,
    pub failed_attempts: u32,
    pub lockout_remaining_ms: u64,
    pub current_app: Option<&'static str>,
    pub volume: u8,
    pub battery_percent: Option<u8>,
    pub wifi_connected: bool,
    pub width: u16,
    pub height: u16,
    pub last_action: String,
    pub pointer_active: bool,
    pub managed_windows: Vec<u32>,
    pub app_launching: bool,
    pub keyboard: PublicKeyboardState,
}

impl ShellState {
    pub fn new(start_locked: bool, initial_volume: u8) -> Self {
        Self {
            view: if start_locked {
                View::Locked
            } else {
                View::Launcher
            },
            screen_awake: true,
            pin_input: Vec::new(),
            failed_attempts: 0,
            lockout_until: None,
            volume: initial_volume.min(100),
            volume_overlay_until: None,
            battery_percent: None,
            wifi_connected: false,
            pointer: None,
            last_action: "startup".into(),
            redraw: true,
            should_exit: false,
            managed_windows: Vec::new(),
            keyboard: KeyboardState::default(),
            app_launch_started: None,
            app_window_mapped_at: None,
        }
    }

    pub fn public(&self, width: u16, height: u16) -> PublicState {
        let remaining = self
            .lockout_until
            .map(|deadline| deadline.saturating_duration_since(Instant::now()))
            .unwrap_or_default();
        PublicState {
            version: env!("CARGO_PKG_VERSION"),
            view: self.view,
            locked: self.view == View::Locked,
            screen_awake: self.screen_awake,
            pin_digits: self.pin_input.len(),
            failed_attempts: self.failed_attempts,
            lockout_remaining_ms: remaining.as_millis().min(u64::MAX as u128) as u64,
            current_app: match self.view {
                View::System => Some("System"),
                View::Browser => Some("Browser"),
                View::Locked | View::Launcher => None,
            },
            volume: self.volume,
            battery_percent: self.battery_percent,
            wifi_connected: self.wifi_connected,
            width,
            height,
            last_action: self.last_action.clone(),
            pointer_active: self.pointer.is_some(),
            managed_windows: self.managed_windows.clone(),
            app_launching: self.app_launching(),
            keyboard: self.keyboard.public(),
        }
    }

    pub fn lock(&mut self) {
        self.keyboard.hide();
        self.view = View::Locked;
        self.clear_pin();
        self.pointer = None;
        self.cancel_app_launch();
        self.last_action = "lock".into();
        self.redraw = true;
    }

    pub fn screen_off(&mut self) {
        self.lock();
        self.screen_awake = false;
        self.last_action = "screen_off".into();
    }

    pub fn screen_on(&mut self) {
        self.lock();
        self.screen_awake = true;
        self.last_action = "screen_on".into();
    }

    pub fn toggle_screen(&mut self) {
        if self.screen_awake {
            self.screen_off();
        } else {
            self.screen_on();
        }
    }

    pub fn home(&mut self) {
        if self.screen_awake && self.view != View::Locked {
            self.keyboard.hide();
            self.view = View::Launcher;
            self.cancel_app_launch();
            self.last_action = "home".into();
            self.redraw = true;
        }
    }

    pub fn launch_system(&mut self) {
        if self.screen_awake && self.view != View::Locked && self.view != View::System {
            self.keyboard.hide();
            self.view = View::System;
            self.managed_windows.clear();
            self.cancel_app_launch();
            self.last_action = "launch_system".into();
            self.redraw = true;
        }
    }

    pub fn launch_browser(&mut self) {
        if self.screen_awake && self.view != View::Locked && self.view != View::Browser {
            self.keyboard.hide();
            self.view = View::Browser;
            self.begin_app_launch();
            self.last_action = "launch_browser".into();
            self.redraw = true;
        }
    }

    pub fn input_digit(&mut self, digit: u8, config: &Config) {
        if !self.screen_awake || self.view != View::Locked || digit > 9 || self.is_lockout_active()
        {
            return;
        }
        if self.pin_input.len() < config.pin_length {
            self.pin_input.push(b'0' + digit);
            self.last_action = "pin_digit".into();
            self.redraw = true;
        }
        if self.pin_input.len() == config.pin_length {
            self.submit_pin(config);
        }
    }

    pub fn backspace_pin(&mut self) {
        if self.screen_awake && self.view == View::Locked && !self.is_lockout_active() {
            self.pin_input.pop();
            self.last_action = "pin_backspace".into();
            self.redraw = true;
        }
    }

    pub fn submit_pin(&mut self, config: &Config) -> bool {
        if !self.screen_awake || self.view != View::Locked || self.is_lockout_active() {
            return false;
        }
        let accepted = verify_pin(config, &self.pin_input);
        self.clear_pin();
        if accepted {
            self.view = View::Launcher;
            self.failed_attempts = 0;
            self.lockout_until = None;
            self.last_action = "unlock".into();
        } else {
            self.failed_attempts = self.failed_attempts.saturating_add(1);
            self.last_action = "unlock_failed".into();
            if self.failed_attempts % 5 == 0 {
                self.lockout_until = Some(Instant::now() + Duration::from_secs(30));
            }
        }
        self.redraw = true;
        accepted
    }

    fn clear_pin(&mut self) {
        self.pin_input.fill(0);
        self.pin_input.clear();
    }

    pub fn is_lockout_active(&mut self) -> bool {
        if let Some(deadline) = self.lockout_until {
            if Instant::now() < deadline {
                return true;
            }
            self.lockout_until = None;
            self.redraw = true;
        }
        false
    }

    pub fn change_volume(&mut self, delta: i8) {
        self.volume = (i16::from(self.volume) + i16::from(delta)).clamp(0, 100) as u8;
        self.volume_overlay_until = Some(Instant::now() + Duration::from_millis(1800));
        self.last_action = if delta >= 0 {
            "volume_up"
        } else {
            "volume_down"
        }
        .into();
        self.redraw = true;
    }

    pub fn set_volume(&mut self, value: u8) {
        self.volume = value.min(100);
        self.volume_overlay_until = Some(Instant::now() + Duration::from_millis(1800));
        self.last_action = "volume_set".into();
        self.redraw = true;
    }

    pub fn show_keyboard(&mut self, purpose: KeyboardPurpose) -> bool {
        if !self.screen_awake
            || !self.view.is_app()
            || self.app_launching()
            || self.managed_windows.is_empty()
        {
            self.last_action = "keyboard_show_blocked".into();
            return false;
        }
        self.keyboard.show(purpose);
        self.last_action = "keyboard_show".into();
        true
    }

    pub fn hide_keyboard(&mut self) {
        if self.keyboard.hide() {
            self.last_action = "keyboard_hide".into();
        }
    }

    pub fn activate_keyboard_key(&mut self, action: KeyAction) -> KeyboardEffect {
        let effect = self.keyboard.activate(action);
        self.last_action = match action {
            KeyAction::Shift => "keyboard_shift",
            KeyAction::SwitchLayout(_) => "keyboard_layout",
            KeyAction::Enter => "keyboard_submit",
            KeyAction::Character(_) | KeyAction::Space | KeyAction::Backspace => "keyboard_input",
        }
        .into();
        effect
    }

    pub fn note_managed_window_closed(&mut self, window: u32) {
        if self.managed_windows.first().copied() == Some(window) {
            self.hide_keyboard();
        }
        self.managed_windows.retain(|managed| *managed != window);
        if self.managed_windows.is_empty() {
            self.hide_keyboard();
        }
    }

    pub fn update_device_status(&mut self, battery_percent: Option<u8>, wifi_connected: bool) {
        let battery_percent = battery_percent.map(|value| value.min(100));
        if self.battery_percent != battery_percent || self.wifi_connected != wifi_connected {
            self.battery_percent = battery_percent;
            self.wifi_connected = wifi_connected;
            self.redraw = true;
        }
    }

    pub fn app_launching(&self) -> bool {
        self.view == View::Browser && self.app_launch_started.is_some()
    }

    pub fn app_launch_elapsed(&self) -> Duration {
        self.app_launch_started
            .map(|started| started.elapsed())
            .unwrap_or_default()
    }

    pub fn note_app_window_mapped(&mut self) {
        if self.app_launching() && self.app_window_mapped_at.is_none() {
            self.app_window_mapped_at = Some(Instant::now());
            self.last_action = match self.view {
                View::System => "system_window_warming",
                View::Browser => "browser_window_warming",
                View::Locked | View::Launcher => return,
            }
            .into();
            self.redraw = true;
        }
    }

    pub fn app_ready_to_reveal(&self) -> bool {
        let Some(mapped_at) = self.app_window_mapped_at else {
            return false;
        };
        self.view == View::Browser && mapped_at.elapsed() >= Duration::from_millis(1500)
    }

    pub fn finish_app_launch(&mut self) {
        if !self.app_launching() {
            return;
        }
        self.app_launch_started = None;
        self.app_window_mapped_at = None;
        if self.view != View::Browser {
            return;
        }
        self.last_action = "browser_ready".into();
        self.redraw = true;
    }

    fn begin_app_launch(&mut self) {
        self.managed_windows.clear();
        self.app_launch_started = Some(Instant::now());
        self.app_window_mapped_at = None;
    }

    fn cancel_app_launch(&mut self) {
        self.app_launch_started = None;
        self.app_window_mapped_at = None;
    }

    pub fn tick(&mut self) {
        let _ = self.is_lockout_active();
        if self
            .volume_overlay_until
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            self.volume_overlay_until = None;
            self.redraw = true;
        }
    }
}

impl Drop for ShellState {
    fn drop(&mut self) {
        self.clear_pin();
    }
}

fn verify_pin(config: &Config, candidate: &[u8]) -> bool {
    let mut hasher = Sha256::new();
    hasher.update(&config.pin_salt);
    hasher.update(candidate);
    let actual = hasher.finalize();
    actual.as_slice().ct_eq(config.pin_hash.as_slice()).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> Config {
        let salt = b"test-salt-123456".to_vec();
        let mut hasher = Sha256::new();
        hasher.update(&salt);
        hasher.update(b"654321");
        Config {
            pin_salt: salt,
            pin_hash: hasher.finalize().to_vec(),
            pin_length: 6,
            start_locked: true,
            initial_volume: 50,
            socket_path: "/tmp/a26-shell-test.sock".into(),
        }
    }

    #[test]
    fn correct_pin_unlocks() {
        let config = test_config();
        let mut state = ShellState::new(true, 50);
        for digit in [6, 5, 4, 3, 2, 1] {
            state.input_digit(digit, &config);
        }
        assert_eq!(state.view, View::Launcher);
    }

    #[test]
    fn wrong_pin_stays_locked() {
        let config = test_config();
        let mut state = ShellState::new(true, 50);
        for digit in [1, 2, 3, 4, 5, 6] {
            state.input_digit(digit, &config);
        }
        assert_eq!(state.view, View::Locked);
        assert_eq!(state.failed_attempts, 1);
    }

    #[test]
    fn power_cycle_wakes_only_to_lock_screen() {
        let config = test_config();
        let mut state = ShellState::new(true, 50);
        for digit in [6, 5, 4, 3, 2, 1] {
            state.input_digit(digit, &config);
        }
        state.launch_system();
        assert_eq!(state.view, View::System);

        state.screen_off();
        assert!(!state.screen_awake);
        assert_eq!(state.view, View::Locked);
        state.screen_on();
        assert!(state.screen_awake);
        assert_eq!(state.view, View::Locked);
    }

    #[test]
    fn browser_is_a_first_class_external_app() {
        let mut state = ShellState::new(false, 50);
        state.launch_browser();
        assert_eq!(state.view, View::Browser);
        assert_eq!(state.public(1080, 2340).current_app, Some("Browser"));
        assert!(state.view.is_app());
        assert!(state.app_launching());

        state.note_app_window_mapped();
        state.app_window_mapped_at = Some(Instant::now() - Duration::from_secs(2));
        assert!(state.app_ready_to_reveal());
        state.finish_app_launch();
        assert!(!state.app_launching());
    }

    #[test]
    fn fast_system_app_does_not_enter_launch_overlay() {
        let mut state = ShellState::new(false, 50);
        state.launch_system();
        assert_eq!(state.view, View::System);
        assert!(!state.app_launching());
        assert!(!state.public(1080, 2340).app_launching);
    }

    #[test]
    fn keyboard_state_follows_app_and_security_transitions() {
        let mut state = ShellState::new(false, 50);
        state.launch_system();
        state.managed_windows.push(42);
        assert!(state.show_keyboard(KeyboardPurpose::Text));
        assert!(state.keyboard.is_visible());

        state.home();
        assert_eq!(state.view, View::Launcher);
        assert!(!state.keyboard.is_visible());

        state.launch_system();
        state.managed_windows.push(43);
        assert!(state.show_keyboard(KeyboardPurpose::Search));
        state.note_managed_window_closed(43);
        assert!(!state.keyboard.is_visible());

        state.managed_windows.push(44);
        assert!(state.show_keyboard(KeyboardPurpose::Password));
        state.lock();
        assert_eq!(state.view, View::Locked);
        assert!(!state.keyboard.is_visible());
    }

    #[test]
    fn password_keyboard_public_state_has_no_typed_content() {
        let mut state = ShellState::new(false, 50);
        state.launch_system();
        state.managed_windows.push(42);
        assert!(state.show_keyboard(KeyboardPurpose::Password));
        assert!(matches!(
            state.activate_keyboard_key(KeyAction::Character('x')),
            KeyboardEffect::Inject(crate::keyboard::KeyboardInput::Character('x'))
        ));

        let public = serde_json::to_value(state.public(1080, 2340)).unwrap();
        assert_eq!(
            public.get("keyboard").unwrap(),
            &serde_json::json!({
                "visible": true,
                "purpose": "password",
                "shift": false,
                "layout": "letters"
            })
        );
        assert_eq!(state.last_action, "keyboard_input");
    }

    #[test]
    fn keyboard_request_and_app_launch_are_blocked_on_lock() {
        let mut state = ShellState::new(true, 50);
        state.managed_windows.push(42);
        assert!(!state.show_keyboard(KeyboardPurpose::Url));
        state.launch_browser();
        state.launch_system();
        assert_eq!(state.view, View::Locked);
        assert!(!state.keyboard.is_visible());
        assert_eq!(state.public(1080, 2340).current_app, None);
        assert_eq!(state.last_action, "keyboard_show_blocked");
    }
}
