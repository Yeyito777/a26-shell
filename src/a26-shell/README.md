# a26-shell

`a26-shell` is the first native phone shell/window manager for the Galaxy A26
research target. Its core UI is toolkit-free: X11 protocol work uses pure-Rust
`x11rb`, and drawing uses X11 rectangles and an embedded bitmap font. Apps are
separate managed processes maintained in their own repositories.

Current features:

- fullscreen numeric lock screen;
- salted, hashed six-digit PIN configuration (no plaintext PIN in source);
- touch-first launcher containing separate **System** and **Browser** apps;
- app-provided System artwork with a one-pixel launcher frame;
- external `a26-system` process launch and fullscreen lifecycle management;
- external ARM64 vimbrowser launch and fullscreen lifecycle management;
- hidden X11 cursor on touch-only shell and app surfaces;
- bottom-edge swipe-up app close gesture;
- volume-key handling and volume overlay;
- physical power-key lock/screen blanking with panel-backlight control;
- ordinary X11 `MapRequest`/`ConfigureRequest` handling;
- root-only Unix-socket IPC for state inspection and deterministic input;
- host-side build/install/test scripts over ADB.

The standalone System app lives at
<https://github.com/Yeyito777/a26-system>. It installs its executable and icon
under `/opt/a26-system`; the shell does not contain the System renderer or
telemetry implementation.

The standalone Browser app is vimbrowser. Its A26 installation provides
`/opt/vimbrowser-a26/bin/vimbrowser-a26` and
`/opt/vimbrowser-a26/share/browser-app.bgrx`; the shell only owns the launcher
tile and process lifecycle.

The lock screen is a UI/session lock, not a cryptographic security boundary.
The unlocked bootloader, Magisk root and authorized ADB can all bypass it by
design. IPC access is restricted to the root-owned chroot runtime.

Runtime paths on the phone:

```text
/opt/a26-shell/bin/a26-shell
/opt/a26-shell/bin/a26-shellctl
/opt/a26-shell/bin/a26-shellshot
/etc/a26-shell/config.json
/run/a26-shell/control.sock
/root/a26-shell.log
```

Useful host controls include `state`, `lock`, `screen on`, `screen off`,
`launch system`, `launch browser`, `swipe-up`, `volume up`, and pointer/tap injection. They are
exposed through `scripts/a26-shell/ctl.sh` and the root-only Unix socket.
`scripts/a26-shell/screenshot.sh` captures the actual X11 shell window over
ADB without relying on Android SurfaceFlinger.

Development lifecycle from the project root:

```sh
# Install/reprovision without writing the PIN into source or argv.
read -rsp 'A26 Shell PIN: ' A26_SHELL_PIN; echo
export A26_SHELL_PIN
scripts/a26-shell/install.sh
unset A26_SHELL_PIN

# Start the complete safe native session, inspect it, and return to Android.
scripts/a26-shell/desktop-start.sh
scripts/a26-shell/ctl.sh state
scripts/a26-shell/desktop-stop.sh
```
