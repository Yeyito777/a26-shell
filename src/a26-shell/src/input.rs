use std::fs::{self, File, OpenOptions};
use std::io::{self, ErrorKind, Read};
use std::mem::size_of;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const EV_KEY: u16 = 0x01;
const KEY_POWER: u16 = 116;
const KEY_PRESSED: i32 = 1;

/// Non-grabbing reader for the PMIC power key. Android does not expose this
/// node to Xorg in the native session, so the shell reads it directly.
pub struct PowerKey {
    file: File,
    pending: Vec<u8>,
    last_press: Option<Instant>,
}

impl PowerKey {
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(path)?;
        Ok(Self {
            file,
            pending: Vec::new(),
            last_press: None,
        })
    }

    pub fn poll_presses(&mut self) -> io::Result<usize> {
        let mut bytes = [0_u8; 24 * 16];
        loop {
            match self.file.read(&mut bytes) {
                Ok(0) => break,
                Ok(count) => self.pending.extend_from_slice(&bytes[..count]),
                Err(error) if error.kind() == ErrorKind::WouldBlock => break,
                Err(error) => return Err(error),
            }
        }

        let timeval_bytes = size_of::<libc::timeval>();
        let event_bytes = timeval_bytes + 8;
        let complete = self.pending.len() / event_bytes * event_bytes;
        let mut presses = 0;
        for event in self.pending[..complete].chunks_exact(event_bytes) {
            let kind = u16::from_ne_bytes([event[timeval_bytes], event[timeval_bytes + 1]]);
            let code = u16::from_ne_bytes([event[timeval_bytes + 2], event[timeval_bytes + 3]]);
            let value = i32::from_ne_bytes([
                event[timeval_bytes + 4],
                event[timeval_bytes + 5],
                event[timeval_bytes + 6],
                event[timeval_bytes + 7],
            ]);
            if kind == EV_KEY && code == KEY_POWER && value == KEY_PRESSED {
                let now = Instant::now();
                if self
                    .last_press
                    .is_none_or(|last| now.duration_since(last) >= Duration::from_millis(350))
                {
                    self.last_press = Some(now);
                    presses += 1;
                }
            }
        }
        self.pending.drain(..complete);
        Ok(presses)
    }
}

/// Samsung's downstream touchscreen requires paired display-state and event
/// values rather than a boolean. `2,1` is the completed DISPLAY_STATE_ON event;
/// `1,0` is the early DISPLAY_STATE_OFF event for this exact firmware.
pub struct TouchscreenPower {
    enabled_path: PathBuf,
}

impl TouchscreenPower {
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let enabled_path = path.as_ref().to_path_buf();
        fs::metadata(&enabled_path)?;
        Ok(Self { enabled_path })
    }

    pub fn off(&self) -> io::Result<()> {
        fs::write(&self.enabled_path, b"1,0\n")
    }

    pub fn on(&self) -> io::Result<()> {
        fs::write(&self.enabled_path, b"2,1\n")
    }
}

/// Minimal panel-backlight port. The shell draws the lock/black frame before
/// changing brightness so unlocked content cannot flash while waking.
pub struct Backlight {
    brightness_path: PathBuf,
    saved_brightness: u16,
}

impl Backlight {
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let brightness_path = path.as_ref().to_path_buf();
        let current_brightness = fs::read_to_string(&brightness_path)?
            .trim()
            .parse::<u16>()
            .map_err(|_| io::Error::new(ErrorKind::InvalidData, "invalid backlight brightness"))?;
        let max_brightness = fs::read_to_string("/sys/class/backlight/panel/max_brightness")
            .ok()
            .and_then(|value| value.trim().parse::<u16>().ok())
            .unwrap_or(128);
        // If a previous development process died while the display was off,
        // wake at a conservative known-good level instead of remaining black.
        let saved_brightness = if current_brightness == 0 {
            128.min(max_brightness).max(1)
        } else {
            current_brightness
        };
        Ok(Self {
            brightness_path,
            saved_brightness,
        })
    }

    pub fn off(&mut self) -> io::Result<()> {
        if let Ok(current) = fs::read_to_string(&self.brightness_path) {
            if let Ok(value) = current.trim().parse::<u16>() {
                if value > 0 {
                    self.saved_brightness = value;
                }
            }
        }
        fs::write(&self.brightness_path, b"0\n")
    }

    pub fn on(&self) -> io::Result<()> {
        fs::write(
            &self.brightness_path,
            format!("{}\n", self.saved_brightness),
        )
    }
}
