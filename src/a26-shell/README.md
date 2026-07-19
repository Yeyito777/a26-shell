# a26-shell

`a26-shell` is the first native phone shell/window manager for the Galaxy A26
research target. It is intentionally self-contained and toolkit-free. X11
protocol work uses pure-Rust `x11rb`; drawing uses X11 rectangles and an
embedded bitmap font.

Current features:

- fullscreen numeric lock screen;
- salted, hashed six-digit PIN configuration (no plaintext PIN in source);
- touch-first launcher containing the **System** app;
- generated System app artwork with a one-pixel launcher frame;
- built-in System information view;
- live CPU/GPU utilization and used/remaining filesystem space;
- bottom-edge swipe-up app close gesture;
- volume-key handling and volume overlay;
- physical power-key lock/screen blanking with panel-backlight control;
- ordinary X11 `MapRequest`/`ConfigureRequest` handling;
- root-only Unix-socket IPC for state inspection and deterministic input;
- host-side build/install/test scripts over ADB.

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
`launch system`, `swipe-up`, `volume up`, and pointer/tap injection. They are
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
