# 0257: what a cold run costs, and the profile that shows it

Status: built (2026-11-15). Amends the profile (`RERAIL_PROFILE`, `profile::report`) and its roofline (O139).

## Intro

The first run after a rebuild cost far more than the next: 59 to 64 G instructions and 5 to 9 s of user CPU on dl10,
against 8 G and 0.5 s. Its profile showed 1.6 s of process CPU, so most of the cost was where no span looked. The
user asked that the cold runs be explained, and that the profile gain what it needs to show this (2026-11-15).

## What a cold run is

A copy of the engine binary at a new path reproduces it, with the stores unchanged. So what is cold is the binary
itself, and two different costs hide behind one number:

| Run (dl10, `layer-violations.rr`) | Wall | User CPU | Instructions |
|---|---|---|---|
| A new binary's first run, no profile | 12.0 to 13.0 s | 0.52 to 0.61 s | 7.7 to 8.0 G |
| The same binary's second run | 0.85 s | 0.52 s | 7.7 G |
| A new binary's first run, profiled | 18.6 to 36 s | 6.6 to 11 s | 46 to 80 G |

- **Before `main`: the system checks a new binary.** A new binary's first exec of `--version` takes 14.7 s of wall
  and no CPU; the second takes 0.01 s. During it `syspolicyd` (Gatekeeper's assessment) is busy. The debug engine is
  415 MB, ad-hoc signed and carries `com.apple.provenance`. This is wall time only, outside the engine. A terminal
  marked as a developer tool (System Settings, Privacy & Security, Developer Tools) is exempt from this check.
- **The profile's roofline calibration.** Its loops ran again for every new binary, because the roof cache was keyed
  by the binary's path, size and time: 9.7 s of CPU in the report, after the CPU the profile counted. A run without
  the profile pays nothing of it.

## Decision

- **The profile's report opens with the process's life** (`profile::life`), what no span sees:
  - the wall from the process's start (as the system records it) to `main` (`tools::watch` marks it);
  - the wall from `main` to the report;
  - the process's CPU, every thread, and that of the child processes it waited for;
  - the report's own work after it (the roofline's calibration), its wall and CPU.

  On the cold profiled run above it reads 16.60 s before `main`, 9.51 s to the report, 2.49 s of the process's CPU
  and 9.70 s of the calibration: all of `time`'s CPU. The roofline's line names the calibration's CPU too.
- **The roof cache is the loops' and the build's, not the binary's.** Its file is named by the loops' code (the
  roofline's own source), the build's profile (`target/<profile>`) and its debug assertions. It stays per machine
  (the temporary directory) and day. A rebuild keeps the roofs.

## What this gives up

- A change of compiler, or of the profile's settings under one profile name, keeps the roofs measured before it for
  up to a day. They are estimates of the machine.

## Tests

- `a_processs_life_names_its_start_its_main_and_its_children` (profile): the start before `main`, a busy loop's
  CPU, and a child process's CPU apart.
- `the_roof_cache_is_the_builds_profile_not_its_binary` (profile).
