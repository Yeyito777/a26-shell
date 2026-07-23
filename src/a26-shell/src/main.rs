mod audio;
mod config;
mod font;
mod input;
mod ipc;
mod keyboard;
mod model;
mod status;
mod ui;
mod volume;

use std::env;
use std::error::Error;
use std::path::PathBuf;
use std::process::{Child, Command as ProcessCommand, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use audio::AudioVolume;
use config::Config;
use input::{Backlight, PowerKey, TouchscreenPower, VolumeKey, VolumeKeys};
use ipc::{Command, IpcServer};
use keyboard::{KeyboardEffect, KeyboardGeometry, KeyboardSurface, XtestInjector};
use model::{PointerGesture, ShellState, View};
use ui::{KeypadAction, Renderer};
use volume::VolumeSurface;
use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xfixes::ConnectionExt as _;
use x11rb::protocol::xinput::{self, ConnectionExt as _};
use x11rb::protocol::xproto::{
    AtomEnum, BUTTON_PRESS_EVENT, BUTTON_RELEASE_EVENT, Blanking, ChangeWindowAttributesAux,
    ConfigureWindowAux, ConnectionExt as _, CreateGCAux, CreateWindowAux, EventMask, Exposures,
    InputFocus, MOTION_NOTIFY_EVENT, PropMode, StackMode, WindowClass,
};
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;
use x11rb::{COPY_DEPTH_FROM_PARENT, CURRENT_TIME};

const DEFAULT_SYSTEM_APP: &str = "/opt/a26-system/bin/a26-system";
const DEFAULT_BROWSER_APP: &str = "/opt/vimbrowser-a26/bin/vimbrowser-a26";
const DEVICE_STATUS_INTERVAL: Duration = Duration::from_secs(5);
const LAUNCH_ANIMATION_INTERVAL: Duration = Duration::from_millis(180);

#[derive(Default)]
struct RawTouchTracker {
    touch_id: Option<u32>,
    x: i16,
    y: i16,
}

fn main() -> Result<(), Box<dyn Error>> {
    let config_path = parse_config_path()?;
    let config = Config::load(&config_path)?;
    let (conn, screen_number) = RustConnection::connect(None)?;
    let screen = &conn.setup().roots[screen_number];
    let root = screen.root;
    let width = screen.width_in_pixels;
    let height = screen.height_in_pixels;

    let root_mask = EventMask::SUBSTRUCTURE_REDIRECT
        | EventMask::SUBSTRUCTURE_NOTIFY
        | EventMask::STRUCTURE_NOTIFY
        | EventMask::PROPERTY_CHANGE;
    conn.change_window_attributes(
        root,
        &ChangeWindowAttributesAux::new().event_mask(root_mask),
    )?
    .check()
    .map_err(|error| format!("cannot become window manager (is another WM running?): {error}"))?;

    let xi_version = conn.xinput_xi_query_version(2, 2)?.reply()?;
    if (xi_version.major_version, xi_version.minor_version) < (2, 2) {
        return Err(format!(
            "XInput 2.2 required; server negotiated {}.{}",
            xi_version.major_version, xi_version.minor_version
        )
        .into());
    }
    conn.xfixes_query_version(4, 0)?.reply()?;
    let mut key_injector = XtestInjector::query(
        &conn,
        root,
        conn.setup().min_keycode,
        conn.setup().max_keycode,
    )?;
    // A direct PMIC power-key read is invisible to the X server. Disable Xorg's
    // independent blanking/DPMS timers so it cannot blank while our shell still
    // believes the screen is awake (volume keys previously appeared to be the
    // only reliable wake because they happened to be X events).
    conn.set_screen_saver(0, 0, Blanking::DEFAULT, Exposures::DEFAULT)?
        .check()?;
    let raw_touch_mask = xinput::XIEventMask::RAW_TOUCH_BEGIN
        | xinput::XIEventMask::RAW_TOUCH_UPDATE
        | xinput::XIEventMask::RAW_TOUCH_END;
    conn.xinput_xi_select_events(
        root,
        &[xinput::EventMask {
            deviceid: xinput::Device::ALL.into(),
            mask: vec![raw_touch_mask],
        }],
    )?
    .check()?;

    let shell_window = conn.generate_id()?;
    let window_aux = CreateWindowAux::new()
        .background_pixel(0x0b1020)
        .override_redirect(1)
        .event_mask(
            EventMask::EXPOSURE
                | EventMask::BUTTON_PRESS
                | EventMask::BUTTON_RELEASE
                | EventMask::POINTER_MOTION
                | EventMask::KEY_PRESS
                | EventMask::STRUCTURE_NOTIFY,
        );
    conn.create_window(
        COPY_DEPTH_FROM_PARENT,
        shell_window,
        root,
        0,
        0,
        width,
        height,
        0,
        WindowClass::INPUT_OUTPUT,
        0,
        &window_aux,
    )?;
    conn.change_property8(
        PropMode::REPLACE,
        root,
        AtomEnum::WM_NAME,
        AtomEnum::STRING,
        b"moon",
    )?;
    conn.change_property8(
        PropMode::REPLACE,
        shell_window,
        AtomEnum::WM_NAME,
        AtomEnum::STRING,
        b"moon-shell",
    )?;

    let shell_back_buffer = conn.generate_id()?;
    conn.create_pixmap(
        screen.root_depth,
        shell_back_buffer,
        shell_window,
        width,
        height,
    )?;
    let gc = conn.generate_id()?;
    conn.create_gc(gc, shell_window, &CreateGCAux::new().graphics_exposures(0))?;
    let keyboard_geometry = KeyboardGeometry::new(width, height);
    let mut keyboard_surface =
        KeyboardSurface::create(&conn, root, screen.root_depth, keyboard_geometry)?;
    let mut volume_surface = VolumeSurface::create(&conn, root, screen.root_depth)?;
    conn.map_window(shell_window)?;
    conn.xfixes_hide_cursor(shell_window)?.check()?;
    conn.xfixes_hide_cursor(keyboard_surface.window)?.check()?;
    conn.xfixes_hide_cursor(volume_surface.window)?.check()?;
    raise_shell(&conn, shell_window)?;
    conn.set_input_focus(InputFocus::PARENT, shell_window, CURRENT_TIME)?;
    conn.flush()?;

    let renderer = Renderer {
        window: shell_window,
        back_buffer: shell_back_buffer,
        gc,
        width,
        height,
        system_icon: ui::load_system_icon(),
        browser_icon: ui::load_browser_icon(),
    };
    let ipc = IpcServer::bind(&config.socket_path)?;
    let audio_volume = match AudioVolume::open("/run/moon-audio/volume") {
        Ok(control) => Some(control),
        Err(error) => {
            eprintln!("audio volume control unavailable: {error}");
            None
        }
    };
    let initial_volume = audio_volume
        .as_ref()
        .and_then(|control| match control.get() {
            Ok(volume) => Some(volume),
            Err(error) => {
                eprintln!("persisted audio volume unavailable: {error}");
                None
            }
        })
        .unwrap_or(config.initial_volume);
    let mut state = ShellState::new(config.start_locked, initial_volume);
    if let Some(control) = audio_volume.as_ref()
        && let Err(error) = control.set(state.volume)
    {
        eprintln!("initial audio volume sync failed: {error}");
    }
    let initial_status = status::DeviceStatus::read();
    state.update_device_status(
        initial_status.battery_percent,
        initial_status.wifi_connected,
    );
    let mut next_device_status = Instant::now() + DEVICE_STATUS_INTERVAL;
    let mut next_launch_animation = Instant::now();
    let mut power_key = match PowerKey::open("/dev/input/event1") {
        Ok(device) => {
            eprintln!("physical power key ready at /dev/input/event1");
            Some(device)
        }
        Err(error) => {
            eprintln!("physical power key unavailable: {error}");
            None
        }
    };
    let mut volume_keys = match VolumeKeys::open("/dev/input/event0") {
        Ok(device) => {
            eprintln!("physical volume keys ready at /dev/input/event0");
            Some(device)
        }
        Err(error) => {
            eprintln!("physical volume keys unavailable: {error}");
            None
        }
    };
    let mut backlight = match Backlight::open("/sys/class/backlight/panel/brightness") {
        Ok(device) => Some(device),
        Err(error) => {
            eprintln!("panel backlight control unavailable: {error}");
            None
        }
    };
    let touchscreen = match TouchscreenPower::open("/sys/class/sec/tsp/enabled") {
        Ok(device) => Some(device),
        Err(error) => {
            eprintln!("touchscreen power control unavailable: {error}");
            None
        }
    };
    let mut hardware_awake = true;
    let mut raw_touch = RawTouchTracker::default();
    let mut system_app: Option<Child> = None;
    let mut browser_app: Option<Child> = None;
    let mut app_viewport: Option<(u32, u16)> = None;
    renderer.render(&conn, &state)?;
    if let Some(device) = touchscreen.as_ref()
        && let Err(error) = device.on()
    {
        eprintln!("initial touchscreen wake failed: {error}");
    }
    if let Some(device) = backlight.as_ref() {
        if let Err(error) = device.on() {
            eprintln!("initial panel wake failed: {error}");
        }
    }

    eprintln!(
        "a26-shell ready display={} root=0x{root:08x} window=0x{shell_window:08x} size={width}x{height} socket={} pid={}",
        env::var("DISPLAY").unwrap_or_else(|_| "(default)".into()),
        config.socket_path.display(),
        std::process::id(),
    );

    while !state.should_exit {
        while let Some(event) = conn.poll_for_event()? {
            handle_x_event(
                &conn,
                event,
                root,
                shell_window,
                keyboard_surface.window,
                volume_surface.window,
                (width, height),
                &keyboard_geometry,
                &config,
                &mut state,
                &mut raw_touch,
                &mut key_injector,
                &mut volume_surface,
            )?;
        }

        if let Some(device) = power_key.as_mut() {
            match device.poll_presses() {
                Ok(count) => {
                    if count > 0 {
                        state.toggle_screen();
                    }
                }
                Err(error) => {
                    eprintln!("physical power key failed: {error}");
                    power_key = None;
                }
            }
        }

        if let Some(device) = volume_keys.as_mut() {
            match device.poll() {
                Ok(keys) => {
                    for key in keys {
                        change_volume(
                            &mut state,
                            audio_volume.as_ref(),
                            match key {
                                VolumeKey::Down => -5,
                                VolumeKey::Up => 5,
                            },
                        );
                    }
                }
                Err(error) => {
                    eprintln!("physical volume keys failed: {error}");
                    volume_keys = None;
                }
            }
        }

        for (stream, request) in ipc.accept_all() {
            match request {
                Ok(command) => {
                    apply_command(
                        &conn,
                        root,
                        command,
                        width,
                        height,
                        &keyboard_geometry,
                        &config,
                        &mut state,
                        &key_injector,
                        audio_volume.as_ref(),
                    );
                    let public = state.public(width, height);
                    ipc::respond(stream, Ok(&public));
                }
                Err(error) => ipc::respond::<model::PublicState>(stream, Err(&error)),
            }
        }

        reconcile_apps(&mut state, &mut system_app, &mut browser_app);
        if state.app_ready_to_reveal() {
            let app_window = state.managed_windows.first().copied();
            state.finish_app_launch();
            if let Some(window) = app_window {
                fullscreen_window(&conn, window, width, height)?;
                conn.set_input_focus(InputFocus::PARENT, window, CURRENT_TIME)?;
            }
        }
        let keyboard_app_window =
            if state.screen_awake && state.view.is_app() && !state.app_launching() {
                state.managed_windows.first().copied()
            } else {
                None
            };
        let desired_viewport = keyboard_app_window.map(|window| {
            (
                window,
                if state.keyboard.is_visible() {
                    keyboard_geometry.app_height()
                } else {
                    height
                },
            )
        });
        if desired_viewport != app_viewport {
            if let Some((window, app_height)) = desired_viewport {
                resize_app_window(&conn, window, width, app_height)?;
                if state.keyboard.is_visible() {
                    state.keyboard.request_raise();
                }
            }
            app_viewport = desired_viewport;
        }
        keyboard_surface.sync(&conn, &mut state.keyboard, keyboard_app_window)?;
        let volume_visible = state.screen_awake
            && state.view.is_app()
            && !state.app_launching()
            && state
                .volume_overlay_until
                .is_some_and(|deadline| deadline > Instant::now());
        volume_surface.sync(&conn, volume_visible, state.volume)?;
        if Instant::now() >= next_device_status {
            let device_status = status::DeviceStatus::read();
            state.update_device_status(device_status.battery_percent, device_status.wifi_connected);
            next_device_status = Instant::now() + DEVICE_STATUS_INTERVAL;
        }
        if state.app_launching() && Instant::now() >= next_launch_animation {
            state.redraw = true;
            next_launch_animation = Instant::now() + LAUNCH_ANIMATION_INTERVAL;
        }
        state.tick();
        if state.screen_awake != hardware_awake {
            // Draw the safe frame before changing brightness. During wake the
            // lock screen is therefore complete before the panel lights up.
            renderer.render(&conn, &state)?;
            state.redraw = false;
            if state.screen_awake {
                if let Some(device) = touchscreen.as_ref()
                    && let Err(error) = device.on()
                {
                    eprintln!("touchscreen wake failed: {error}");
                }
                if let Some(device) = backlight.as_mut()
                    && let Err(error) = device.on()
                {
                    eprintln!("panel backlight wake failed: {error}");
                    backlight = None;
                }
            } else {
                if let Some(device) = backlight.as_mut()
                    && let Err(error) = device.off()
                {
                    eprintln!("panel backlight sleep failed: {error}");
                    backlight = None;
                }
                if let Some(device) = touchscreen.as_ref()
                    && let Err(error) = device.off()
                {
                    eprintln!("touchscreen sleep failed: {error}");
                }
            }
            hardware_awake = state.screen_awake;
        }
        if state.redraw {
            if !state.view.is_app() || state.app_launching() {
                raise_shell(&conn, shell_window)?;
                renderer.render(&conn, &state)?;
            }
            state.redraw = false;
        }
        // External-app MapRequest/configure operations may be the only X11
        // traffic in this state, so they cannot rely on a shell repaint to
        // flush the connection.
        conn.flush()?;
        thread::sleep(Duration::from_millis(8));
    }

    // A normal development restart must never strand the device with its
    // backlight at zero. Wake only to the freshly rendered lock screen.
    if !state.screen_awake {
        state.screen_on();
        let _ = renderer.render(&conn, &state);
        if let Some(device) = touchscreen.as_ref() {
            let _ = device.on();
        }
        if let Some(device) = backlight.as_ref() {
            let _ = device.on();
        }
    }
    stop_system_app(&mut system_app);
    stop_system_app(&mut browser_app);
    let _ = conn.destroy_window(shell_window);
    keyboard_surface.destroy(&conn);
    volume_surface.destroy(&conn);
    let _ = conn.free_pixmap(shell_back_buffer);
    let _ = conn.free_gc(gc);
    let _ = conn.flush();
    eprintln!("a26-shell stopping");
    Ok(())
}

fn parse_config_path() -> Result<PathBuf, Box<dyn Error>> {
    let mut arguments = env::args().skip(1);
    let mut config = env::var_os("A26_SHELL_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/etc/a26-shell/config.json".into());
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--config" => config = arguments.next().ok_or("--config requires a path")?.into(),
            "--help" | "-h" => {
                println!("usage: a26-shell [--config PATH]");
                std::process::exit(0);
            }
            _ => return Err(format!("unknown argument: {argument}").into()),
        }
    }
    Ok(config)
}

#[allow(clippy::too_many_arguments)]
fn handle_x_event(
    conn: &RustConnection,
    event: Event,
    root: u32,
    shell_window: u32,
    keyboard_window: u32,
    volume_window: u32,
    dimensions: (u16, u16),
    keyboard_geometry: &KeyboardGeometry,
    config: &Config,
    state: &mut ShellState,
    raw_touch: &mut RawTouchTracker,
    key_injector: &mut XtestInjector,
    volume_surface: &mut VolumeSurface,
) -> Result<(), Box<dyn Error>> {
    let (width, height) = dimensions;
    match event {
        Event::XinputRawTouchBegin(event) => {
            if raw_touch.touch_id.is_none() || state.pointer.is_none() {
                let x = raw_axis(&event, 0, width.saturating_sub(1));
                let y = raw_axis(&event, 1, height.saturating_sub(1));
                if let (Some(x), Some(y)) = (x, y) {
                    raw_touch.touch_id = Some(event.detail);
                    raw_touch.x = x;
                    raw_touch.y = y;
                    pointer_begin(state, keyboard_geometry, x, y);
                }
            }
        }
        Event::XinputRawTouchUpdate(event) if raw_touch.touch_id == Some(event.detail) => {
            if let Some(x) = raw_axis(&event, 0, width.saturating_sub(1)) {
                raw_touch.x = x;
            }
            if let Some(y) = raw_axis(&event, 1, height.saturating_sub(1)) {
                raw_touch.y = y;
            }
            pointer_move(state, raw_touch.x, raw_touch.y);
        }
        Event::XinputRawTouchEnd(event) if raw_touch.touch_id == Some(event.detail) => {
            pointer_end(
                conn,
                root,
                state,
                raw_touch.x,
                raw_touch.y,
                width,
                height,
                keyboard_geometry,
                config,
                key_injector,
            );
            raw_touch.touch_id = None;
        }
        Event::Expose(event) if event.window == shell_window => state.redraw = true,
        Event::Expose(event) if event.window == keyboard_window => {
            state.keyboard.request_redraw();
        }
        Event::Expose(event) if event.window == volume_window => {
            volume_surface.request_redraw();
        }
        Event::ButtonPress(event) if event.event == shell_window => {
            pointer_begin(state, keyboard_geometry, event.event_x, event.event_y);
        }
        Event::MotionNotify(event) if event.event == shell_window => {
            pointer_move(state, event.event_x, event.event_y);
        }
        Event::ButtonRelease(event) if event.event == shell_window => {
            pointer_end(
                conn,
                root,
                state,
                event.event_x,
                event.event_y,
                width,
                height,
                keyboard_geometry,
                config,
                key_injector,
            );
        }
        Event::MapRequest(event) => {
            if event.window != shell_window
                && event.window != keyboard_window
                && event.window != volume_window
            {
                conn.map_window(event.window)?;
                conn.xfixes_hide_cursor(event.window)?.check()?;
                if !state.managed_windows.contains(&event.window) {
                    state.managed_windows.push(event.window);
                }
                let is_primary = state.managed_windows.first().copied() == Some(event.window);
                state.last_action = "map_external_window".into();
                state.redraw = true;
                if state.view.is_app() {
                    let app_height = if is_primary && state.keyboard.is_visible() {
                        keyboard_geometry.app_height()
                    } else {
                        height
                    };
                    fullscreen_window(conn, event.window, width, app_height)?;
                    if state.app_launching() {
                        if is_primary {
                            state.note_app_window_mapped();
                        }
                        raise_shell(conn, shell_window)?;
                    } else if is_primary {
                        conn.set_input_focus(InputFocus::PARENT, event.window, CURRENT_TIME)?;
                    }
                    state.keyboard.request_raise();
                } else {
                    raise_shell(conn, shell_window)?;
                }
            }
        }
        Event::ConfigureRequest(event) => {
            if state.view.is_app() && state.managed_windows.contains(&event.window) {
                let is_primary = state.managed_windows.first().copied() == Some(event.window);
                if is_primary && state.keyboard.is_visible() {
                    resize_app_window(conn, event.window, width, keyboard_geometry.app_height())?;
                } else {
                    fullscreen_window(conn, event.window, width, height)?;
                }
                state.keyboard.request_raise();
                if state.app_launching() {
                    raise_shell(conn, shell_window)?;
                }
            } else {
                let aux = ConfigureWindowAux::from_configure_request(&event);
                conn.configure_window(event.window, &aux)?;
                // Browser engines create auxiliary clipboard, selection, and
                // popup windows. Their configure requests must not raise the
                // shell over the application's primary window.
                if state.view.is_app() {
                    state.keyboard.request_raise();
                } else {
                    raise_shell(conn, shell_window)?;
                }
            }
        }
        Event::MapNotify(event)
            if event.window != shell_window
                && event.window != keyboard_window
                && event.window != volume_window =>
        {
            state.keyboard.request_raise();
            volume_surface.request_redraw();
        }
        Event::ConfigureNotify(event)
            if event.window != shell_window
                && event.window != keyboard_window
                && event.window != volume_window =>
        {
            state.keyboard.request_raise();
            volume_surface.request_redraw();
        }
        Event::DestroyNotify(event) => {
            if event.window != volume_window {
                state.note_managed_window_closed(event.window);
            }
        }
        Event::UnmapNotify(event) => {
            if event.window != keyboard_window && event.window != volume_window {
                state.note_managed_window_closed(event.window);
            }
        }
        Event::MappingNotify(_) => {
            if let Err(error) = key_injector.refresh(conn) {
                eprintln!("cannot refresh X keyboard mapping: {error}");
            }
        }
        _ => {}
    }
    Ok(())
}

fn raw_axis(event: &xinput::RawTouchBeginEvent, wanted: usize, maximum: u16) -> Option<i16> {
    let mut value_index = 0;
    for (word_index, mask) in event.valuator_mask.iter().copied().enumerate() {
        for bit in 0..32 {
            if mask & (1_u32 << bit) == 0 {
                continue;
            }
            let axis = word_index * 32 + bit;
            let value = event.axisvalues_raw.get(value_index)?;
            if axis == wanted {
                return Some(value.integral.clamp(0, i32::from(maximum)) as i16);
            }
            value_index += 1;
        }
    }
    None
}

fn raise_shell(conn: &RustConnection, shell_window: u32) -> Result<(), Box<dyn Error>> {
    conn.configure_window(
        shell_window,
        &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
    )?;
    Ok(())
}

fn fullscreen_window(
    conn: &RustConnection,
    window: u32,
    width: u16,
    height: u16,
) -> Result<(), Box<dyn Error>> {
    conn.configure_window(
        window,
        &ConfigureWindowAux::new()
            .x(0)
            .y(0)
            .width(u32::from(width))
            .height(u32::from(height))
            .border_width(0)
            .stack_mode(StackMode::ABOVE),
    )?;
    Ok(())
}

fn resize_app_window(
    conn: &RustConnection,
    window: u32,
    width: u16,
    height: u16,
) -> Result<(), Box<dyn Error>> {
    conn.configure_window(
        window,
        &ConfigureWindowAux::new()
            .x(0)
            .y(0)
            .width(u32::from(width))
            .height(u32::from(height.max(1)))
            .border_width(0),
    )?;
    Ok(())
}

fn reconcile_apps(
    state: &mut ShellState,
    system_child: &mut Option<Child>,
    browser_child: &mut Option<Child>,
) {
    match state.view {
        View::System => {
            stop_system_app(browser_child);
            let executable = env::var_os("A26_SYSTEM_APP")
                .map(PathBuf::from)
                .unwrap_or_else(|| DEFAULT_SYSTEM_APP.into());
            reconcile_app(state, system_child, &executable, "System");
        }
        View::Browser => {
            stop_system_app(system_child);
            let executable = env::var_os("A26_BROWSER_APP")
                .map(PathBuf::from)
                .unwrap_or_else(|| DEFAULT_BROWSER_APP.into());
            reconcile_app(state, browser_child, &executable, "Browser");
        }
        View::Locked | View::Launcher => {
            stop_system_app(system_child);
            stop_system_app(browser_child);
        }
    }
}

fn reconcile_app(
    state: &mut ShellState,
    child: &mut Option<Child>,
    executable: &PathBuf,
    name: &str,
) {
    if let Some(process) = child.as_mut() {
        match process.try_wait() {
            Ok(Some(status)) => {
                eprintln!("{name} exited with {status}");
                *child = None;
                state.home();
                state.last_action = format!("{}_process_exited", name.to_ascii_lowercase());
            }
            Ok(None) => return,
            Err(error) => {
                eprintln!("cannot inspect {name}: {error}");
                stop_system_app(child);
                state.home();
                state.last_action = format!("{}_process_error", name.to_ascii_lowercase());
            }
        }
    }

    if !state.view.is_app() || child.is_some() {
        return;
    }
    match ProcessCommand::new(executable)
        .env(
            "DISPLAY",
            env::var("DISPLAY").unwrap_or_else(|_| ":0".into()),
        )
        .stdin(Stdio::null())
        .spawn()
    {
        Ok(process) => {
            eprintln!("started {name} pid={}", process.id());
            *child = Some(process);
            state.last_action = format!("{}_process_started", name.to_ascii_lowercase());
        }
        Err(error) => {
            eprintln!("cannot start {}: {error}", executable.display());
            state.home();
            state.last_action = format!("{}_launch_failed", name.to_ascii_lowercase());
        }
    }
}

fn stop_system_app(child: &mut Option<Child>) {
    if let Some(mut process) = child.take() {
        let _ = process.kill();
        let _ = process.wait();
    }
}

fn pointer_begin(state: &mut ShellState, keyboard_geometry: &KeyboardGeometry, x: i16, y: i16) {
    if !state.screen_awake {
        return;
    }
    let keyboard_owned = keyboard_geometry.owns_touch(&state.keyboard, x, y);
    let keyboard_key_index = keyboard_geometry.key_index_at(&state.keyboard, x, y);
    state.pointer = Some(PointerGesture {
        start_x: x,
        start_y: y,
        last_x: x,
        last_y: y,
        started: Instant::now(),
        keyboard_owned,
        keyboard_key_index,
    });
    state.last_action = "pointer_begin".into();
}

fn pointer_move(state: &mut ShellState, x: i16, y: i16) {
    if let Some(pointer) = state.pointer.as_mut() {
        pointer.last_x = x;
        pointer.last_y = y;
    }
}

#[allow(clippy::too_many_arguments)]
fn pointer_end(
    conn: &RustConnection,
    root: u32,
    state: &mut ShellState,
    x: i16,
    y: i16,
    width: u16,
    height: u16,
    keyboard_geometry: &KeyboardGeometry,
    config: &Config,
    key_injector: &XtestInjector,
) {
    let Some(mut pointer) = state.pointer.take() else {
        return;
    };
    pointer.last_x = x;
    pointer.last_y = y;
    let dx = i32::from(pointer.last_x) - i32::from(pointer.start_x);
    let dy = i32::from(pointer.last_y) - i32::from(pointer.start_y);
    let elapsed = pointer.started.elapsed();
    if pointer.keyboard_owned {
        let end_key_index = keyboard_geometry.key_index_at(&state.keyboard, x, y);
        if dx.abs() <= 60
            && dy.abs() <= 60
            && elapsed <= Duration::from_millis(850)
            && pointer.keyboard_key_index == end_key_index
        {
            if let Some(index) = pointer.keyboard_key_index {
                if let Some(key) = keyboard_geometry.keys(&state.keyboard).get(index) {
                    handle_keyboard_action(conn, state, key.action, key_injector);
                }
            } else {
                state.last_action = "keyboard_tap_between_keys".into();
            }
        } else {
            state.last_action = "keyboard_gesture_cancel".into();
        }
        return;
    }
    let upward = -dy;
    let bottom_start = i32::from(pointer.start_y) >= i32::from(height) - 180;
    let close_swipe = state.view.is_app()
        && bottom_start
        && upward >= 350
        && dx.abs() <= 300
        && upward * 3 >= dx.abs() * 4
        && elapsed <= Duration::from_millis(1400);
    if close_swipe {
        state.home();
        state.last_action = "swipe_up_close".into();
        return;
    }
    if dx.abs() <= 35 && dy.abs() <= 35 && elapsed <= Duration::from_millis(650) {
        handle_tap(
            conn,
            root,
            state,
            x,
            y,
            width,
            keyboard_geometry,
            config,
            key_injector,
        );
    } else {
        state.last_action = "gesture_cancel".into();
    }
    state.redraw = true;
}

#[allow(clippy::too_many_arguments)]
fn handle_tap(
    conn: &RustConnection,
    root: u32,
    state: &mut ShellState,
    x: i16,
    y: i16,
    width: u16,
    keyboard_geometry: &KeyboardGeometry,
    config: &Config,
    key_injector: &XtestInjector,
) {
    if keyboard_geometry.owns_touch(&state.keyboard, x, y) {
        if let Some(action) = keyboard_geometry.action_at(&state.keyboard, x, y) {
            handle_keyboard_action(conn, state, action, key_injector);
        } else {
            state.last_action = "keyboard_tap_between_keys".into();
        }
        return;
    }
    match state.view {
        View::Locked => match ui::keypad_action_at(width, x, y) {
            Some(KeypadAction::Digit(digit)) => state.input_digit(digit, config),
            Some(KeypadAction::Backspace) => state.backspace_pin(),
            Some(KeypadAction::Submit) => {
                state.submit_pin(config);
            }
            None => state.last_action = "lock_tap_outside".into(),
        },
        View::Launcher => {
            if ui::system_app_at(x, y) {
                state.launch_system();
            } else if ui::browser_app_at(x, y) {
                state.launch_browser();
            } else {
                state.last_action = "launcher_tap_outside".into();
            }
        }
        View::System | View::Browser => {
            let app_name = if state.view == View::System {
                "system"
            } else {
                "browser"
            };
            let Some(_window) = state.managed_windows.first().copied() else {
                state.last_action = format!("{app_name}_tap_no_window");
                state.redraw = true;
                return;
            };
            match forward_tap(conn, root, x, y) {
                Ok(()) => state.last_action = format!("{app_name}_tap_forwarded"),
                Err(error) => {
                    eprintln!("cannot forward tap to {app_name}: {error}");
                    state.last_action = format!("{app_name}_tap_failed");
                }
            }
        }
    }
    state.redraw = true;
}

fn handle_keyboard_action(
    conn: &RustConnection,
    state: &mut ShellState,
    action: keyboard::KeyAction,
    key_injector: &XtestInjector,
) {
    match state.activate_keyboard_key(action) {
        KeyboardEffect::Inject(input) => {
            let result = state
                .managed_windows
                .first()
                .copied()
                .ok_or_else(|| "active app window is unavailable".into())
                .and_then(|primary| key_injector.inject(conn, input, primary));
            if let Err(error) = result {
                // Never include the character or key action in this diagnostic;
                // password input is intentionally ephemeral and non-loggable.
                eprintln!("keyboard input injection failed: {error}");
                state.hide_keyboard();
                state.last_action = "keyboard_input_failed".into();
            }
        }
        KeyboardEffect::None => {}
    }
}

#[allow(clippy::too_many_arguments)]
fn apply_command(
    conn: &RustConnection,
    root: u32,
    command: Command,
    width: u16,
    height: u16,
    keyboard_geometry: &KeyboardGeometry,
    config: &Config,
    state: &mut ShellState,
    key_injector: &XtestInjector,
    audio_volume: Option<&AudioVolume>,
) {
    match command {
        Command::Ping | Command::State => {}
        Command::Digit(digit) => state.input_digit(digit, config),
        Command::Backspace => state.backspace_pin(),
        Command::Submit => {
            state.submit_pin(config);
        }
        Command::Tap(x, y) => handle_tap(
            conn,
            root,
            state,
            x,
            y,
            width,
            keyboard_geometry,
            config,
            key_injector,
        ),
        Command::PointerBegin(x, y) => pointer_begin(state, keyboard_geometry, x, y),
        Command::PointerMove(x, y) => pointer_move(state, x, y),
        Command::PointerEnd(x, y) => pointer_end(
            conn,
            root,
            state,
            x,
            y,
            width,
            height,
            keyboard_geometry,
            config,
            key_injector,
        ),
        Command::Lock => state.lock(),
        Command::Home => state.home(),
        Command::LaunchSystem => state.launch_system(),
        Command::LaunchBrowser => state.launch_browser(),
        Command::KeyboardShow(purpose) => {
            state.show_keyboard(purpose);
        }
        Command::KeyboardHide => state.hide_keyboard(),
        Command::SwipeUp => {
            if state.view.is_app() {
                state.home();
                state.last_action = "swipe_up_close".into();
            }
        }
        Command::VolumeUp => change_volume(state, audio_volume, 5),
        Command::VolumeDown => change_volume(state, audio_volume, -5),
        Command::VolumeSet(value) => set_volume(state, audio_volume, value),
        Command::Power => state.toggle_screen(),
        Command::ScreenOff => state.screen_off(),
        Command::ScreenOn => state.screen_on(),
        Command::Quit => state.should_exit = true,
    }
}

fn change_volume(state: &mut ShellState, audio_volume: Option<&AudioVolume>, delta: i8) {
    state.change_volume(delta);
    sync_audio_volume(audio_volume, state.volume);
}

fn set_volume(state: &mut ShellState, audio_volume: Option<&AudioVolume>, volume: u8) {
    state.set_volume(volume);
    sync_audio_volume(audio_volume, state.volume);
}

fn sync_audio_volume(audio_volume: Option<&AudioVolume>, volume: u8) {
    if let Some(control) = audio_volume
        && let Err(error) = control.set(volume)
    {
        eprintln!("audio volume sync failed: {error}");
    }
}

fn forward_tap(conn: &RustConnection, root: u32, x: i16, y: i16) -> Result<(), Box<dyn Error>> {
    // Send through XTEST rather than constructing an event for the managed
    // top-level. Chromium places its page surface in descendant X windows; a
    // SendEvent aimed at the top-level never reaches the renderer and cannot
    // establish page focus. XTEST performs normal server hit-testing at the
    // physical coordinate, so native app controls and embedded page surfaces
    // receive the same pointer sequence as a hardware tap.
    conn.xtest_fake_input(MOTION_NOTIFY_EVENT, 0, CURRENT_TIME, root, x, y, 0)?
        .check()?;
    for response_type in [BUTTON_PRESS_EVENT, BUTTON_RELEASE_EVENT] {
        conn.xtest_fake_input(response_type, 1, CURRENT_TIME, root, 0, 0, 0)?
            .check()?;
    }
    conn.flush()?;
    Ok(())
}

#[cfg(test)]
mod touch_tests {
    use super::*;

    fn fp(value: i32) -> xinput::Fp3232 {
        xinput::Fp3232 {
            integral: value,
            frac: 0,
        }
    }

    #[test]
    fn raw_touch_axes_use_physical_valuator_coordinates() {
        let event = xinput::RawTouchBeginEvent {
            valuator_mask: vec![0b1111],
            axisvalues_raw: vec![fp(820), fp(808), fp(6), fp(4)],
            ..Default::default()
        };
        assert_eq!(raw_axis(&event, 0, 1079), Some(820));
        assert_eq!(raw_axis(&event, 1, 2339), Some(808));
    }

    #[test]
    fn sparse_raw_touch_update_preserves_absent_axis() {
        let event = xinput::RawTouchBeginEvent {
            valuator_mask: vec![0b0010],
            axisvalues_raw: vec![fp(1700)],
            ..Default::default()
        };
        assert_eq!(raw_axis(&event, 0, 1079), None);
        assert_eq!(raw_axis(&event, 1, 2339), Some(1700));
    }
}
