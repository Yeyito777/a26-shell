use std::env;
use std::ffi::CString;
use std::fs;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;

use crate::model::AppId;

const DEFAULT_FREEZER_ROOT: &str = "/dev/freezer";
const MOON_GROUP: &str = "moon";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FreezerState {
    Unavailable,
    Thawed,
    Freezing,
    Frozen,
    Unknown,
}

#[derive(Debug)]
pub struct FreezerGroup {
    path: Option<PathBuf>,
    public_path: Option<String>,
    assigned: bool,
}

impl FreezerGroup {
    pub fn prepare(app: AppId) -> Self {
        let root = env::var_os("A26_FREEZER_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| DEFAULT_FREEZER_ROOT.into());
        match Self::prepare_at(&root, app) {
            Ok(group) => group,
            Err(error) => {
                eprintln!(
                    "{} freezer cgroup unavailable at {}: {error}",
                    app.display_name(),
                    root.display()
                );
                Self {
                    path: None,
                    public_path: None,
                    assigned: false,
                }
            }
        }
    }

    fn prepare_at(root: &Path, app: AppId) -> io::Result<Self> {
        if !root.join("cgroup.procs").is_file() || !root.join("tasks").is_file() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "not a freezer cgroup v1 mount",
            ));
        }
        let moon = root.join(MOON_GROUP);
        let name = app.cgroup_name();
        let path = moon.join(name);
        fs::create_dir_all(&path)?;
        let group = Self {
            path: Some(path),
            public_path: Some(format!("/{MOON_GROUP}/{name}")),
            assigned: false,
        };
        // A process left frozen by a prior abnormal shell exit must never make
        // the replacement shell hang while it inspects or terminates it.
        group.set_state("THAWED")?;
        Ok(group)
    }

    pub fn configure_spawn(&self, command: &mut Command) -> io::Result<()> {
        let Some(path) = self.path.as_ref() else {
            return Ok(());
        };
        let cgroup_procs = path.join("cgroup.procs");
        if !cgroup_procs.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("{} is missing", cgroup_procs.display()),
            ));
        }
        let cgroup_procs = CString::new(cgroup_procs.as_os_str().as_bytes())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "cgroup path contains NUL"))?;
        // SAFETY: this closure runs after fork and before exec. It uses only
        // async-signal-safe libc calls and stack storage; no allocation, locks,
        // formatting, or Rust filesystem code occurs in the child.
        unsafe {
            command.pre_exec(move || assign_current_process(&cgroup_procs));
        }
        Ok(())
    }

    pub fn note_spawned(&mut self) {
        self.assigned = self.path.is_some();
    }

    pub fn note_stopped(&mut self) {
        self.assigned = false;
    }

    pub fn public_path(&self) -> Option<String> {
        self.assigned.then(|| self.public_path.clone()).flatten()
    }

    pub fn state(&self) -> FreezerState {
        if !self.assigned {
            return FreezerState::Unavailable;
        }
        let Some(path) = self.path.as_ref() else {
            return FreezerState::Unavailable;
        };
        match fs::read_to_string(path.join("freezer.state")) {
            Ok(value) => match value.trim() {
                "THAWED" => FreezerState::Thawed,
                "FREEZING" => FreezerState::Freezing,
                "FROZEN" => FreezerState::Frozen,
                _ => FreezerState::Unknown,
            },
            Err(_) => FreezerState::Unavailable,
        }
    }

    fn set_state(&self, state: &str) -> io::Result<()> {
        let Some(path) = self.path.as_ref() else {
            return Ok(());
        };
        fs::write(path.join("freezer.state"), format!("{state}\n"))
    }
}

unsafe fn assign_current_process(cgroup_procs: &CString) -> io::Result<()> {
    // SAFETY: cgroup_procs is a valid NUL-terminated path and libc::open is
    // async-signal-safe.
    let fd = unsafe { libc::open(cgroup_procs.as_ptr(), libc::O_WRONLY | libc::O_CLOEXEC) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }

    let mut bytes = [0_u8; 32];
    // SAFETY: getpid has no preconditions and is async-signal-safe.
    let pid = unsafe { libc::getpid() } as u32;
    let length = encode_pid(pid, &mut bytes);
    let mut written = 0;
    while written < length {
        // SAFETY: the slice is valid for the requested byte count and fd is an
        // open cgroup.procs descriptor.
        let result =
            unsafe { libc::write(fd, bytes[written..length].as_ptr().cast(), length - written) };
        if result < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            // SAFETY: fd is owned by this function.
            unsafe { libc::close(fd) };
            return Err(error);
        }
        written += result as usize;
    }
    // SAFETY: fd is owned by this function.
    let close_result = unsafe { libc::close(fd) };
    if close_result < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn encode_pid(mut pid: u32, destination: &mut [u8; 32]) -> usize {
    let mut cursor = destination.len() - 1;
    destination[cursor] = b'\n';
    if pid == 0 {
        cursor -= 1;
        destination[cursor] = b'0';
    } else {
        while pid > 0 {
            cursor -= 1;
            destination[cursor] = b'0' + (pid % 10) as u8;
            pid /= 10;
        }
    }
    let length = destination.len() - cursor;
    destination.copy_within(cursor.., 0);
    length
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pid_encoding_is_allocation_free_and_newline_terminated() {
        let mut bytes = [0; 32];
        let length = encode_pid(4_294_967_295, &mut bytes);
        assert_eq!(&bytes[..length], b"4294967295\n");
        let length = encode_pid(0, &mut bytes);
        assert_eq!(&bytes[..length], b"0\n");
    }

    #[test]
    fn non_cgroup_root_is_rejected() {
        let root = env::temp_dir().join(format!("moon-freezer-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let error = FreezerGroup::prepare_at(&root, AppId::System).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        fs::remove_dir_all(root).unwrap();
    }
}
