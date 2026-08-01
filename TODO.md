1. [x] Replace per-app child slots with a lifecycle-aware application registry.
2. [x] Make swipe-up hide and background apps instead of terminating them.
3. [x] Assign each app process tree to an isolated freezer cgroup.
4. Unfreeze and remap background apps for instant reopening.
5. Add bounded background-execution leases for media and transfers.
6. Connect Vimbrowser media playback state to Moon’s lease protocol.
7. Add memory-pressure eviction of least-recently-used background apps.
8. Replace Moon’s idle 8 ms polling with blocking event waits.
9. Add a device-local suspend coordinator with safe lock, display, and app sequencing.
10. Enter deep suspend on screen-off and wake safely from approved hardware sources.
11. Add a central alarm and notification service with RTC wake scheduling.
12. Measure and document foreground, background, audio, and suspended battery use.
