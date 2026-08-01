use std::env;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use crate::freezer::{FreezerGroup, FreezerState};
use crate::lease::{LeaseKind, LeaseManager, PublicLeaseState};
use crate::model::{AppId, AppLifecycle, PublicAppState};

const DEFAULT_SYSTEM_APP: &str = "/opt/a26-system/bin/a26-system";
const DEFAULT_BROWSER_APP: &str = "/opt/vimbrowser-a26/bin/vimbrowser-a26";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowVisibility {
    Show(u32),
    Hide(u32),
}

#[derive(Debug, Default)]
pub struct RegistryUpdate {
    pub visibility: Vec<WindowVisibility>,
    pub freeze_after_hide: Vec<AppId>,
    pub resumed: Option<AppId>,
    pub active_process_exited: Option<AppId>,
    pub active_process_failed: Option<AppId>,
}

#[derive(Debug, Clone, Copy)]
struct ManagedWindow {
    id: u32,
    visible: bool,
}

#[derive(Debug)]
struct Application {
    id: AppId,
    executable: PathBuf,
    child: Option<Child>,
    lifecycle: AppLifecycle,
    windows: Vec<ManagedWindow>,
    freezer: FreezerGroup,
}

impl Application {
    fn new(id: AppId, executable: PathBuf) -> Self {
        Self {
            id,
            executable,
            child: None,
            lifecycle: AppLifecycle::Stopped,
            windows: Vec::new(),
            freezer: FreezerGroup::prepare(id),
        }
    }

    fn pid(&self) -> Option<u32> {
        self.child.as_ref().map(Child::id)
    }

    fn public(&self, leases: Vec<PublicLeaseState>) -> PublicAppState {
        PublicAppState {
            app: self.id,
            lifecycle: self.lifecycle,
            pid: self.pid(),
            windows: self.windows.iter().map(|window| window.id).collect(),
            freezer_cgroup: self.freezer.public_path(),
            freezer_state: self.freezer.state(),
            leases,
        }
    }

    fn stop(&mut self) {
        if let Err(error) = self.freezer.thaw() {
            eprintln!(
                "cannot thaw {} while stopping: {error}",
                self.id.display_name()
            );
        }
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.lifecycle = AppLifecycle::Stopped;
        self.windows.clear();
        self.freezer.note_stopped();
    }
}

pub struct AppRegistry {
    applications: [Application; 2],
    active: Option<AppId>,
    leases: LeaseManager,
}

impl AppRegistry {
    pub fn from_environment() -> Self {
        let system = env::var_os("A26_SYSTEM_APP")
            .map(PathBuf::from)
            .unwrap_or_else(|| DEFAULT_SYSTEM_APP.into());
        let browser = env::var_os("A26_BROWSER_APP")
            .map(PathBuf::from)
            .unwrap_or_else(|| DEFAULT_BROWSER_APP.into());
        Self::new(system, browser)
    }

    fn new(system: PathBuf, browser: PathBuf) -> Self {
        Self {
            applications: [
                Application::new(AppId::System, system),
                Application::new(AppId::Browser, browser),
            ],
            active: None,
            leases: LeaseManager::default(),
        }
    }

    pub fn active(&self) -> Option<AppId> {
        self.active
    }

    pub fn public(&self) -> Vec<PublicAppState> {
        let now = Instant::now();
        self.applications
            .iter()
            .map(|application| application.public(self.leases.public(application.id, now)))
            .collect()
    }

    pub fn active_windows(&self) -> Vec<u32> {
        self.active
            .map(|id| {
                self.app(id)
                    .windows
                    .iter()
                    .map(|window| window.id)
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn primary_active_window(&self) -> Option<u32> {
        self.active
            .and_then(|id| self.app(id).windows.first().map(|window| window.id))
    }

    pub fn owner_of(&self, window: u32) -> Option<AppId> {
        self.applications.iter().find_map(|application| {
            application
                .windows
                .iter()
                .any(|managed| managed.id == window)
                .then_some(application.id)
        })
    }

    pub fn is_active_window(&self, window: u32) -> bool {
        self.active
            .is_some_and(|active| self.owner_of(window) == Some(active))
    }

    pub fn register_window(&mut self, owner: AppId, window: u32) -> WindowRegistration {
        let should_show = self.active == Some(owner);
        let application = self.app_mut(owner);
        if let Some(existing) = application
            .windows
            .iter_mut()
            .find(|managed| managed.id == window)
        {
            existing.visible = should_show;
        } else {
            application.windows.push(ManagedWindow {
                id: window,
                visible: should_show,
            });
        }
        if should_show && application.lifecycle == AppLifecycle::Launching {
            application.lifecycle = AppLifecycle::Foreground;
        }
        WindowRegistration {
            owner,
            should_show,
            primary: application.windows.first().map(|managed| managed.id) == Some(window),
        }
    }

    pub fn note_mapped(&mut self, window: u32) -> bool {
        let Some(owner) = self.owner_of(window) else {
            return true;
        };
        let should_show = self.active == Some(owner);
        if let Some(managed) = self
            .app_mut(owner)
            .windows
            .iter_mut()
            .find(|managed| managed.id == window)
        {
            managed.visible = should_show;
        }
        should_show
    }

    pub fn note_unmapped(&mut self, window: u32) {
        let Some(owner) = self.owner_of(window) else {
            return;
        };
        let application = self.app_mut(owner);
        let intentionally_hidden = application
            .windows
            .iter()
            .find(|managed| managed.id == window)
            .is_some_and(|managed| !managed.visible);
        if !intentionally_hidden {
            application.windows.retain(|managed| managed.id != window);
        }
    }

    pub fn remove_window(&mut self, window: u32) {
        for application in &mut self.applications {
            application.windows.retain(|managed| managed.id != window);
        }
    }

    pub fn reconcile(&mut self, desired: Option<AppId>) -> RegistryUpdate {
        let mut update = RegistryUpdate::default();
        let now = Instant::now();
        self.leases.expire(now);
        for application in &mut self.applications {
            let result = application.child.as_mut().map(Child::try_wait);
            match result {
                Some(Ok(Some(status))) => {
                    eprintln!("{} exited with {status}", application.id.display_name());
                    application.child = None;
                    application.lifecycle = AppLifecycle::Stopped;
                    application.windows.clear();
                    application.freezer.note_stopped();
                    self.leases.clear_app(application.id);
                    if desired == Some(application.id) {
                        update.active_process_exited = Some(application.id);
                    }
                }
                Some(Err(error)) => {
                    eprintln!("cannot inspect {}: {error}", application.id.display_name());
                    application.stop();
                    if desired == Some(application.id) {
                        update.active_process_failed = Some(application.id);
                    }
                }
                Some(Ok(None)) | None => {}
            }
        }

        if update.active_process_exited.is_some() || update.active_process_failed.is_some() {
            self.active = None;
            return update;
        }

        // An app with a lease remains thawed while backgrounded. As soon as its
        // final bounded lease expires, the ordinary reconciliation pass puts
        // it back into the freezer even if another app is foreground.
        for id in [AppId::System, AppId::Browser] {
            let needs_freeze = {
                let application = self.app(id);
                desired != Some(id)
                    && application.lifecycle == AppLifecycle::Background
                    && matches!(
                        application.freezer.state(),
                        FreezerState::Thawed | FreezerState::Unknown
                    )
            };
            if needs_freeze && !self.leases.active(id, now) {
                update.freeze_after_hide.push(id);
            }
        }

        if self.active != desired {
            if let Some(previous) = self.active {
                let leased = self.leases.active(previous, now);
                let application = self.app_mut(previous);
                if application.lifecycle != AppLifecycle::Stopped {
                    application.lifecycle = AppLifecycle::Background;
                    for window in &mut application.windows {
                        if window.visible {
                            window.visible = false;
                            update.visibility.push(WindowVisibility::Hide(window.id));
                        }
                    }
                    if !leased {
                        update.freeze_after_hide.push(previous);
                    }
                }
            }
            self.active = desired;
        }

        let Some(active) = desired else {
            return update;
        };
        let application = self.app_mut(active);
        match application.lifecycle {
            AppLifecycle::Background if application.windows.is_empty() => {
                // The process survived, but it no longer owns a reusable
                // top-level window. Treat its next MapRequest as a warm launch
                // rather than claiming that an invisible app was resumed.
                application.lifecycle = AppLifecycle::Launching;
            }
            AppLifecycle::Background => {
                if let Err(error) = application.freezer.thaw() {
                    eprintln!("cannot resume {}: {error}", application.id.display_name());
                    application.stop();
                    update.active_process_failed = Some(active);
                    self.active = None;
                    return update;
                }
                application.lifecycle = AppLifecycle::Foreground;
                for window in &mut application.windows {
                    if !window.visible {
                        window.visible = true;
                        update.visibility.push(WindowVisibility::Show(window.id));
                    }
                }
                update.resumed = Some(active);
            }
            AppLifecycle::Stopped => {
                let mut command = Command::new(&application.executable);
                command
                    .env(
                        "DISPLAY",
                        env::var("DISPLAY").unwrap_or_else(|_| ":0".into()),
                    )
                    .stdin(Stdio::null());
                if let Err(error) = application.freezer.configure_spawn(&mut command) {
                    eprintln!(
                        "cannot isolate {} before launch: {error}",
                        application.id.display_name()
                    );
                    update.active_process_failed = Some(active);
                    self.active = None;
                    return update;
                }
                match command.spawn() {
                    Ok(process) => {
                        eprintln!(
                            "started {} pid={}",
                            application.id.display_name(),
                            process.id()
                        );
                        application.child = Some(process);
                        application.freezer.note_spawned();
                        application.lifecycle = AppLifecycle::Launching;
                    }
                    Err(error) => {
                        eprintln!("cannot start {}: {error}", application.executable.display());
                        update.active_process_failed = Some(active);
                        self.active = None;
                    }
                }
            }
            AppLifecycle::Launching | AppLifecycle::Foreground => {}
        }
        update
    }

    pub fn shutdown(&mut self) {
        for application in &mut self.applications {
            application.stop();
        }
        self.active = None;
        self.leases = LeaseManager::default();
    }

    pub fn freeze_background(&mut self, id: AppId) -> std::io::Result<()> {
        let application = self.app_mut(id);
        if application.lifecycle == AppLifecycle::Background {
            application.freezer.freeze()?;
        }
        Ok(())
    }

    pub fn acquire_lease(
        &mut self,
        app: AppId,
        kind: LeaseKind,
        seconds: u64,
        now: Instant,
    ) -> Result<Duration, &'static str> {
        if self.app(app).lifecycle == AppLifecycle::Stopped {
            return Err("cannot lease a stopped app");
        }
        let duration = self.leases.acquire(app, kind, seconds, now)?;
        if self.app(app).lifecycle == AppLifecycle::Background
            && self.app(app).freezer.thaw().is_err()
        {
            self.leases.release(app, kind);
            return Err("cannot thaw leased app");
        }
        Ok(duration)
    }

    pub fn release_lease(
        &mut self,
        app: AppId,
        kind: LeaseKind,
        now: Instant,
    ) -> Result<(), &'static str> {
        self.leases.release(app, kind);
        if self.app(app).lifecycle == AppLifecycle::Background && !self.leases.active(app, now) {
            self.app(app)
                .freezer
                .freeze()
                .map_err(|_| "cannot freeze released app")?;
        }
        Ok(())
    }

    fn app(&self, id: AppId) -> &Application {
        &self.applications[id.index()]
    }

    fn app_mut(&mut self, id: AppId) -> &mut Application {
        &mut self.applications[id.index()]
    }
}

#[derive(Debug, Clone, Copy)]
pub struct WindowRegistration {
    pub owner: AppId,
    pub should_show: bool,
    pub primary: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> AppRegistry {
        AppRegistry::new("/bin/true".into(), "/bin/true".into())
    }

    #[test]
    fn registry_keeps_windows_separate_across_lifecycle_transitions() {
        let mut registry = registry();
        registry.active = Some(AppId::System);
        registry.app_mut(AppId::System).lifecycle = AppLifecycle::Foreground;
        registry.app_mut(AppId::System).child =
            Some(Command::new("sh").args(["-c", "sleep 30"]).spawn().unwrap());
        registry.register_window(AppId::System, 11);

        let update = registry.reconcile(None);
        assert_eq!(update.visibility, vec![WindowVisibility::Hide(11)]);
        assert_eq!(update.freeze_after_hide, vec![AppId::System]);
        assert_eq!(
            registry.app(AppId::System).lifecycle,
            AppLifecycle::Background
        );
        assert_eq!(registry.owner_of(11), Some(AppId::System));

        let update = registry.reconcile(Some(AppId::System));
        assert_eq!(update.resumed, Some(AppId::System));
        assert_eq!(update.visibility, vec![WindowVisibility::Show(11)]);
        assert_eq!(registry.primary_active_window(), Some(11));
        registry.shutdown();
    }

    #[test]
    fn intentional_background_unmap_does_not_forget_window() {
        let mut registry = registry();
        registry.active = Some(AppId::Browser);
        registry.app_mut(AppId::Browser).lifecycle = AppLifecycle::Foreground;
        registry.register_window(AppId::Browser, 22);
        registry.reconcile(None);
        registry.note_unmapped(22);
        assert_eq!(registry.owner_of(22), Some(AppId::Browser));
        registry.remove_window(22);
        assert_eq!(registry.owner_of(22), None);
    }

    #[test]
    fn windowless_background_process_returns_to_launching() {
        let mut registry = registry();
        registry.app_mut(AppId::System).lifecycle = AppLifecycle::Background;
        let update = registry.reconcile(Some(AppId::System));
        assert_eq!(update.resumed, None);
        assert_eq!(
            registry.app(AppId::System).lifecycle,
            AppLifecycle::Launching
        );
    }
}
