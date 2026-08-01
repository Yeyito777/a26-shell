# X11 input deadlock incident

## Captured failure

Moon stopped accepting touch and control commands while its process and Xorg
both remained alive. Before restarting anything, the live process showed:

- one Moon thread sleeping indefinitely in `ppoll()` on its X11 socket;
- an empty client/server socket queue, while fresh X11 clients remained healthy;
- a symbolized native backtrace ending at
  `XtestInjector::inject -> get_input_focus()?.reply()`;
- Xorg itself idle in `epoll_wait()`, not globally hung.

The long-lived Moon X connection emits many unchecked rendering and XTEST
requests. X11 replies carry a 16-bit sequence number. After enough requests, a
synchronous reply on that same connection can be associated with the wrong
sequence epoch and leave the cookie waiting forever. Because injection ran on
Moon's sole event-loop thread, this also blocked touch, power handling, and IPC.

## Fix

Runtime input no longer performs any synchronous X11 operation. Moon selects
the trusted active app from its own registry, tracks focus through X11 focus
events on managed top-levels, preserves the exact descendant/transient focus,
queues the complete XTEST key or tap sequence without checked cookies, and
flushes. `SetInputFocus` is used only for explicit app map/resume transitions.
Synchronous extension/version/mapping checks are restricted to startup, before
the connection can wrap.

The autonomous supervisor now treats a responsive control socket—not a PID—as
the liveness criterion. It uses a hard timeout. Two failed probes create a
root-only incident report containing process state, kernel wait channel/stack,
Xorg state, and Moon's log tail; it then terminates Moon's recorded process group
and restarts locally within a bounded deadline. One local recovery is allowed per
boot, only the newest ten incident reports are retained, and Android is restored
if recovery fails or another incident occurs. No key identity or typed text is
recorded.

## Verification requirements

1. Formatting, unit tests, Clippy with warnings denied, and the static AArch64
   release build must pass.
2. Ordinary keys, Backspace, Enter, hold-repeat, and simultaneous touches must
   work on the real device.
3. During sustained input, repeated `a26-shellctl ping` requests must remain
   responsive and Moon must never sleep in an X11 reply wait.
4. The installed autonomous supervisor must contain the bounded liveness probe
   and incident-capture path.

## Device verification performed

The final focus-preserving build completed 35,000 consecutive Backspace actions
through Moon's real control/input path. Each action queues two XTEST key requests,
so this deliberately crossed a full 16-bit request epoch with at least 70,000
runtime X11 requests. Concurrent liveness probes all succeeded in 334–358 ms
including ADB process overhead, and sampled wait channels remained normal
sleep/active states rather than an X11 socket wait. A fresh managed Browser then
accepted ordinary input after the wrap. A real top-level-to-CEF-descendant focus
transition kept `app_focused=true` and delivered input to the focused DOM field,
while Moon continued answering control requests throughout.
