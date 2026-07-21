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
- system-owned global on-screen keyboard for managed apps, with text, URL,
  search, password, and number purposes;
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
`launch system`, `launch browser`, `keyboard show url`, `keyboard hide`,
`swipe-up`, `volume up`, and pointer/tap injection. They are exposed through
`scripts/a26-shell/ctl.sh` and the root-only Unix socket.
`scripts/a26-shell/screenshot.sh` captures the actual X11 shell window over
ADB without relying on Android SurfaceFlinger.

## On-screen keyboard protocol

The keyboard is Moon system UI, not part of Browser or System. It is a dedicated
lower-screen override-redirect X11 window above the active managed app. Moon's
existing XI2 raw-touch path consumes touches in the key panel before its normal
app tap forwarding. The bottom 180 physical pixels contain no keys and remain
available as the global swipe-to-close start zone.

Managed apps request keyboard state with one command on the existing control
socket, read the JSON response, and close the connection:

```text
keyboard show text
keyboard show url
keyboard show search
keyboard show password
keyboard show number
keyboard hide
```

There is no keyboard event socket. Moon leaves X focus on the app and emits each
key through XTEST using the server keyboard and modifier maps. A short physical
press interval is retained between key down and key up. The Browser target has a
narrow A26-only CEF compatibility path that turns the resulting raw printable key
into the missing renderer CHAR event; no typed value crosses IPC or enters a Moon
buffer. HIDE also emits one Escape after unmapping so clients can end editing and
a later tap on the same field can request the keyboard again.

App taps use XTEST pointer injection as well. This lets the X server hit-test into
embedded descendant windows such as Chromium's page surface instead of sending a
synthetic event only to the managed top-level. A show request is ignored unless
the screen is awake and a managed Browser/System window is active; lock,
screen-off, home, app close, and the global close gesture always hide the
keyboard.

`state` exposes only this keyboard metadata:

```json
{
  "visible": true,
  "purpose": "password",
  "shift": false,
  "layout": "letters"
}
```

Moon never builds a text buffer for app input. In particular, password keys are
resolved and injected one at a time, are never included in diagnostics, and are
never present in public state.

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
