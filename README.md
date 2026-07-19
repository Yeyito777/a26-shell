# A26 Shell

An experimental touch-first X11 phone shell and window manager written in Rust
for the Samsung Galaxy A26.

![A26 Shell launcher](notes/a26-shell/launcher-redesign.png)

A26 Shell runs directly on the phone's native Xorg session. It currently
provides a numeric lock screen, a minimal app launcher, external X11 app
lifecycle management, one-finger app-closing gestures, physical power/volume
key policy, and a root-only Unix-socket control interface over ADB.

> [!WARNING]
> This is research software, not a turnkey custom ROM. The reference setup uses
> an unlocked bootloader, Magisk, Samsung's downstream kernel, a separately
> prepared Alpine rootfs, and a native Xorg DRM/KMS takeover. Flashing or
> unlocking Samsung devices can erase data and permanently trip Knox.

## Highlights

- Rust 2024, toolkit-free UI using pure-Rust `x11rb`
- XInput 2.2 raw-touch handling for the A26 touchscreen
- Salted PIN configuration with constant-time verification
- Power-button lock and panel-backlight control
- Separate-process app launch, fullscreen placement, close, and power-lock lifecycle
- App-provided artwork with an exact one-pixel launcher frame
- Static `aarch64-unknown-linux-musl` deployment binaries
- Root-only JSON IPC and deterministic ADB test controls
- X11-native screenshot capture independent of Android SurfaceFlinger

## Repository layout

```text
src/a26-shell/          Rust window-manager crate
scripts/a26-shell/      build, install, lifecycle, IPC, and test helpers
notes/a26-shell/        curated screenshots and design verification
```

## Build

Host requirements include a current Rust toolchain, the
`aarch64-unknown-linux-musl` target, `aarch64-linux-musl-gcc`, and ADB.

```sh
scripts/a26-shell/build.sh
```

Generated binaries and source archives are written under
`images/a26-shell-0.1.0/` and intentionally ignored by Git.

## Device installation

The helper scripts expect the reference Alpine rootfs at
`/data/local/a26-linux`, authorized root access through
`/data/local/tmp/su`, and a working Xorg display `:0`.

If exactly one authorized ADB device is attached, it is selected
automatically. With multiple devices, set `A26_SERIAL` explicitly.

Provision the six-digit UI PIN without placing it in source or command-line
arguments:

```sh
read -rsp 'A26 Shell PIN: ' A26_SHELL_PIN; echo
export A26_SHELL_PIN
scripts/a26-shell/install.sh
unset A26_SHELL_PIN
```

The launcher’s System app is intentionally maintained and installed from its
own repository:

```text
https://github.com/Yeyito777/a26-system
```

Install `a26-system` first. It supplies both
`/opt/a26-system/bin/a26-system` and the launcher icon under
`/opt/a26-system/share/`.

Start just the shell when Xorg is already running:

```sh
scripts/a26-shell/start.sh
```

The `desktop-start.sh` and `desktop-stop.sh` wrappers integrate with the wider
Galaxy A26 research workspace's native-Xorg takeover scripts, which are not
part of this standalone repository.

## Development control

```sh
scripts/a26-shell/ctl.sh state
scripts/a26-shell/ctl.sh lock
scripts/a26-shell/ctl.sh screen off
scripts/a26-shell/ctl.sh screen on
scripts/a26-shell/ctl.sh launch system
scripts/a26-shell/ctl.sh swipe-up
scripts/a26-shell/ctl.sh volume up
scripts/a26-shell/screenshot.sh
```

The control socket is root-owned and mode `0600`. The lock screen is a session
UI, not a cryptographic security boundary: an unlocked bootloader, root, or an
authorized ADB host can bypass it by design.

## Current limitations

- The initial external-app contract is intentionally small and System-specific;
  a general manifest/discovery protocol is the next launcher milestone.
- Touch taps and one-finger swipes work; multitouch/pinch policy is not yet
  implemented.
- Volume is currently shell policy/UI state; a native application-audio mixer
  backend remains to be added.
- The native Xorg/DRM takeover environment must be prepared separately.

## License

MIT — see [LICENSE](LICENSE).
