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
- focus-independent physical volume-key handling, a global volume overlay, and
  real application-audio gain control;
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
/run/moon-audio/{pcm,volume,bridge.pid}
/root/a26-shell.log
```

Useful host controls include `state`, `lock`, `screen on`, `screen off`,
`launch system`, `launch browser`, `keyboard show url`, `keyboard hide`,
`swipe-up`, `volume up`, and pointer/tap injection. They are exposed through
`scripts/a26-shell/ctl.sh` and the root-only Unix socket.
`scripts/a26-shell/screenshot.sh` captures the composed X11 root screen over ADB
without relying on Android SurfaceFlinger.

## Native-session audio

Samsung's AudioFlinger and audio HAL remain alive when Moon suspends the Android
Java framework, but a new AudioTrack cannot be authorized after PackageManager,
PermissionManager, and AudioService stop. The autonomous supervisor therefore
starts a small system-UID Java bridge before the DRM handoff. Its already-created
48 kHz stereo AudioTrack remains valid after `system_server` exits.

Linux applications write signed 16-bit PCM to the root/system-only
`/run/moon-audio/pcm` FIFO. Moon writes its 0–100 gain to the adjacent volume
file. The physical GPIO volume keys are read directly from `/dev/input/event0`,
so their behavior does not depend on which app or descendant X11 window has
focus. A dedicated override-redirect volume surface provides feedback above
both System and Browser. Browser's private rootfs sees only this FIFO; it still
does not receive the phone's `/dev/snd` or unsafe camera devices.

## On-screen keyboard protocol

The keyboard is Moon system UI, not part of Browser or System. It is a dedicated
lower-screen override-redirect X11 window above the active managed app. Moon's
existing XI2 raw-touch path consumes touches in the key panel before its normal
app tap forwarding. The bottom 180 physical pixels contain no keys and remain
available as the global swipe-to-close start zone.

English (US) uses the familiar four-row iPhone portrait arrangement: staggered
QWERTY letter rows, wider Shift/Delete controls, standard `123` and `#+=`
layers, a dedicated number pad, and contextual Done/Search/Go keys. URL entry
uses the Safari-style `123`, Space, `.`, Go bottom row. Moon preserves its own
visual language while scaling geometry proportionally from a 390-point iPhone
width reference. Apple does not publish fixed system-keyboard rectangles;
measurements and source references are recorded in
`notes/a26-shell/APPLE-KEYBOARD-LAYOUT.md`.

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
buffer. A managed app can dismiss the keyboard explicitly over IPC or by ending
its editable-field focus; Moon also hides it on every security/app transition.

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

`install.sh` also installs the AudioTrack bridge and the chroot/runtime helpers
required by `desktop-start.sh`; enabling autonomous boot remains a separate,
explicit `install-autostart.sh` operation.

### Autonomous boot

The phone can boot into Moon without a host-side ADB command. Android and the
Samsung vendor services still start first because they initialize and decrypt
the hardware environment used by Xorg. A Magisk late-start service then hands
DRM/input/network ownership to the native session and launches Moon:

```sh
scripts/a26-shell/install-autostart.sh
scripts/a26-shell/autostart.sh status
```

Safety policy:

- wait for `sys.boot_completed`, `/dev/dri/card0`, and input initialization;
- remain in Android after a kernel-panic reset or after three failed starts;
- require at least 20% battery before takeover;
- restore Android charging mode at 8%;
- verify Xorg and Moon using phone-local status/control paths;
- authorize and verify the AudioTrack bridge before suspending system_server;
- clean native Wi-Fi before restoring Android;
- press a volume key during the eight-second override window to skip Moon once;
- use `autostart.sh skip-once|disable|enable` for explicit host control.
