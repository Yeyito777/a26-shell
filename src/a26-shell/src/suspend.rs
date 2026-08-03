use std::ffi::c_char;
use std::fs;
use std::io;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use serde::Serialize;

const DEVICE_SCRIPT: &str = "/proc/1/root/data/adb/moon/moon-suspend-cycle.sh";
const RUNTIME_DIR: &str = "/run/moon-suspend";
const STATE_FILE: &str = "/run/moon-suspend/state";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SuspendPhase {
    Awake,
    PreparingSleep,
    ScreenOff,
    Suspending,
    Waking,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuspendAction {
    None,
    Sleep,
    Wake,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PersistedSuspendState {
    pub count: u64,
    pub last_suspend_ms: Option<u64>,
    pub last_error: Option<String>,
}

impl PersistedSuspendState {
    fn read(path: &Path) -> io::Result<Self> {
        let value = match fs::read_to_string(path) {
            Ok(value) => value,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => return Err(error),
        };
        let mut state = Self::default();
        for line in value.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            match key {
                "count" => {
                    state.count = value.parse().map_err(|_| {
                        io::Error::new(io::ErrorKind::InvalidData, "invalid suspend count")
                    })?;
                }
                "last_suspend_ms" if !value.is_empty() => {
                    state.last_suspend_ms = Some(value.parse().map_err(|_| {
                        io::Error::new(io::ErrorKind::InvalidData, "invalid suspend duration")
                    })?);
                }
                "last_error" if !value.is_empty() => {
                    if value.len() > 80
                        || !value
                            .bytes()
                            .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
                    {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "invalid suspend error",
                        ));
                    }
                    state.last_error = Some(value.to_owned());
                }
                _ => {}
            }
        }
        Ok(state)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PublicSuspendState {
    pub phase: SuspendPhase,
    pub hardware_awake: bool,
    pub deep_available: bool,
    pub deep_inhibited: bool,
    pub deep_suspend_count: u64,
    pub last_suspend_ms: Option<u64>,
    pub last_error: Option<String>,
    pub transitions: u64,
}

#[derive(Debug)]
pub struct SuspendCoordinator {
    phase: SuspendPhase,
    hardware_awake: bool,
    deep_available: bool,
    deep_inhibited: bool,
    deep_suspend_count: u64,
    last_suspend_ms: Option<u64>,
    test_wake_after: Option<Duration>,
    last_error: Option<String>,
    transitions: u64,
}

impl SuspendCoordinator {
    pub fn new(
        hardware_awake: bool,
        deep_available: bool,
        persisted: PersistedSuspendState,
    ) -> Self {
        Self {
            phase: if hardware_awake {
                SuspendPhase::Awake
            } else {
                SuspendPhase::ScreenOff
            },
            hardware_awake,
            deep_available,
            deep_inhibited: false,
            deep_suspend_count: persisted.count,
            last_suspend_ms: persisted.last_suspend_ms,
            test_wake_after: None,
            last_error: persisted.last_error,
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
        self.deep_inhibited = false;
        self.last_error = None;
        self.transitions = self.transitions.saturating_add(1);
    }

    pub fn fail_sleep(&mut self, code: &'static str) {
        self.hardware_awake = true;
        self.phase = SuspendPhase::Awake;
        self.last_error = Some(code.to_owned());
    }

    pub fn fail_wake(&mut self, code: &'static str) {
        self.hardware_awake = true;
        self.phase = SuspendPhase::Awake;
        self.deep_inhibited = false;
        self.last_error = Some(code.to_owned());
    }

    pub fn screen_is_off(&self) -> bool {
        self.phase == SuspendPhase::ScreenOff && !self.hardware_awake
    }

    pub fn set_deep_inhibited(&mut self, inhibited: bool) {
        self.deep_inhibited = inhibited;
    }

    pub fn arm_test_wake(&mut self, after: Duration) {
        self.test_wake_after = Some(after);
    }

    pub fn take_test_wake(&mut self) -> Option<Duration> {
        self.test_wake_after.take()
    }

    pub fn begin_deep_suspend(&mut self) {
        self.phase = SuspendPhase::Suspending;
        self.deep_inhibited = false;
    }

    #[cfg(test)]
    pub fn complete_deep_suspend(&mut self, elapsed: Duration) {
        self.phase = SuspendPhase::Waking;
        self.deep_suspend_count = self.deep_suspend_count.saturating_add(1);
        self.last_suspend_ms = Some(elapsed.as_millis().min(u64::MAX as u128) as u64);
    }

    pub fn public(&self) -> PublicSuspendState {
        PublicSuspendState {
            phase: self.phase,
            hardware_awake: self.hardware_awake,
            deep_available: self.deep_available,
            deep_inhibited: self.deep_inhibited,
            deep_suspend_count: self.deep_suspend_count,
            last_suspend_ms: self.last_suspend_ms,
            last_error: self.last_error.clone(),
            transitions: self.transitions,
        }
    }
}

/// Starts the device-namespace suspend cycle and then lets this X11 session end.
/// The helper quiesces DSI/DPU before requesting Samsung's deep state, because
/// attempting deep suspend with an active Xorg CRTC provably triggers the
/// platform watchdog in `pmucal_local_disable`. After resume it records the
/// result and performs a controlled warm reboot: this firmware can neither turn
/// an Xorg-disabled DSI CRTC back on in place nor reattach Exynos DWC3 reliably.
/// Autonomous startup then presents a fresh, locked Moon session.
#[derive(Debug)]
pub struct PlatformSuspend {
    script_path: PathBuf,
    state_path: PathBuf,
}

impl PlatformSuspend {
    pub fn open() -> io::Result<(Self, PersistedSuspendState)> {
        Self::open_at(
            Path::new(DEVICE_SCRIPT),
            Path::new(RUNTIME_DIR),
            Path::new(STATE_FILE),
        )
    }

    fn open_at(
        script_path: &Path,
        runtime_dir: &Path,
        state_path: &Path,
    ) -> io::Result<(Self, PersistedSuspendState)> {
        let metadata = fs::metadata(script_path)?;
        if !metadata.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "suspend helper is not a file",
            ));
        }
        if !runtime_dir.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "suspend runtime is unavailable",
            ));
        }
        let persisted = PersistedSuspendState::read(state_path)?;
        Ok((
            Self {
                script_path: script_path.to_path_buf(),
                state_path: state_path.to_path_buf(),
            },
            persisted,
        ))
    }

    pub fn request(&self, test_wake_after: Option<Duration>) -> io::Result<()> {
        // Read once immediately before handoff. A removed runtime bind or helper
        // must fail while Moon can still roll the display transaction back.
        fs::metadata(&self.script_path)?;
        let _ = PersistedSuspendState::read(&self.state_path)?;

        let session_id = unsafe { libc::getsid(0) };
        if session_id <= 0 {
            return Err(io::Error::last_os_error());
        }
        let test_seconds = test_wake_after
            .map(|duration| duration.as_secs().clamp(2, 30))
            .unwrap_or(0);
        let script = self
            .script_path
            .strip_prefix("/proc/1/root")
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid helper root"))?;

        let mut command = Command::new("/system/bin/sh");
        command
            .arg(script)
            .arg(std::process::id().to_string())
            .arg(session_id.to_string())
            .arg(test_seconds.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        // SAFETY: this closure uses only async-signal-safe libc calls between
        // fork and exec. It escapes the Alpine chroot through Android init's
        // root, creates an independent session, and then execs Android sh.
        unsafe {
            command.pre_exec(|| {
                static INIT_ROOT: &[u8] = b"/proc/1/root\0";
                static ROOT: &[u8] = b"/\0";
                if libc::chroot(INIT_ROOT.as_ptr().cast::<c_char>()) < 0 {
                    return Err(io::Error::last_os_error());
                }
                if libc::chdir(ROOT.as_ptr().cast::<c_char>()) < 0 {
                    return Err(io::Error::last_os_error());
                }
                if libc::setsid() < 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        command.spawn()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn coordinator_has_explicit_sleep_suspend_and_persisted_resume() {
        let persisted = PersistedSuspendState {
            count: 2,
            last_suspend_ms: Some(4200),
            last_error: None,
        };
        let mut coordinator = SuspendCoordinator::new(true, true, persisted);
        assert_eq!(coordinator.next_action(false), SuspendAction::Sleep);
        assert_eq!(coordinator.public().phase, SuspendPhase::PreparingSleep);
        coordinator.complete_sleep();
        assert_eq!(coordinator.public().phase, SuspendPhase::ScreenOff);
        coordinator.begin_deep_suspend();
        assert_eq!(coordinator.public().phase, SuspendPhase::Suspending);
        coordinator.complete_deep_suspend(Duration::from_secs(5));
        assert_eq!(coordinator.public().phase, SuspendPhase::Waking);
        coordinator.complete_wake();
        assert_eq!(coordinator.public().phase, SuspendPhase::Awake);
        assert_eq!(coordinator.public().deep_suspend_count, 3);
        assert_eq!(coordinator.public().last_suspend_ms, Some(5000));
        assert_eq!(coordinator.public().transitions, 2);
    }

    #[test]
    fn failed_sleep_rolls_back_to_an_awake_stable_state() {
        let mut coordinator = SuspendCoordinator::new(true, true, PersistedSuspendState::default());
        assert_eq!(coordinator.next_action(false), SuspendAction::Sleep);
        coordinator.fail_sleep("touchscreen_sleep_failed");
        let state = coordinator.public();
        assert_eq!(state.phase, SuspendPhase::Awake);
        assert!(state.hardware_awake);
        assert_eq!(
            state.last_error.as_deref(),
            Some("touchscreen_sleep_failed")
        );
    }

    #[test]
    fn persisted_state_parser_is_bounded_and_fail_closed() {
        let root = env::temp_dir().join(format!("moon-suspend-state-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let state = root.join("state");
        fs::write(
            &state,
            b"version=1\ncount=7\nlast_suspend_ms=9123\nlast_error=\n",
        )
        .unwrap();
        assert_eq!(
            PersistedSuspendState::read(&state).unwrap(),
            PersistedSuspendState {
                count: 7,
                last_suspend_ms: Some(9123),
                last_error: None,
            }
        );
        fs::write(&state, b"count=oops\n").unwrap();
        assert!(PersistedSuspendState::read(&state).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn platform_port_requires_a_helper_and_runtime() {
        let root = env::temp_dir().join(format!("moon-suspend-port-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("run")).unwrap();
        let script = root.join("helper.sh");
        let state = root.join("run/state");
        fs::write(&script, b"#!/bin/sh\n").unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(&state, b"count=1\nlast_suspend_ms=2000\nlast_error=\n").unwrap();
        let (_, persisted) = PlatformSuspend::open_at(&script, &root.join("run"), &state).unwrap();
        assert_eq!(persisted.count, 1);
        assert_eq!(persisted.last_suspend_ms, Some(2000));
        fs::remove_file(script).unwrap();
        assert!(
            PlatformSuspend::open_at(&root.join("helper.sh"), &root.join("run"), &state).is_err()
        );
        fs::remove_dir_all(root).unwrap();
    }
}
