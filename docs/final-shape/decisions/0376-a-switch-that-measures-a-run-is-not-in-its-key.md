# 0376. A switch that measures a run is not in its key

Date: 2026-11-17. Status: decided; built.

## Intro

A run's cutoff (t23 §4, `corpus::reuse`) keeps a run's outputs under a key of what the run is a function of. The key
takes every `RERAIL_*` switch, less a list of the ones that only measure or hold the run (`NOT_OUTPUTS`). A
measurement's cold runs passed the heap sampler's switches (`RERAIL_HEAP_SAMPLE`, `RERAIL_HEAP_AT`) and its warm
runs did not. Every warm run then looked its record up under a key no cold run wrote, missed the cutoff, and ran
phase 1 again: about 9 s of CPU instead of 0.2 s.

## Decided

- The switches that measure a run are left out of its key, as the profile's and the traces' already were: the heap
  sampler's (`RERAIL_HEAP_SAMPLE`, `RERAIL_HEAP_AT`, `RERAIL_HEAP_FRAMES`), the profile's (`RERAIL_PROF`,
  `RERAIL_PROF_MEMORY_MS`, `RERAIL_TIMELINE`), the stores a measurement keeps apart (`RERAIL_COSTS`,
  `RERAIL_IMAGE_STORE`, `RERAIL_EXPORT_IMAGES`: a plan's costs and a program's stored form, never its outputs), and
  `RERAIL_FRAMES_TRACE`.
- `reuse::measures_only` is the one test of whether a switch is in the key.
- Two tests keep it from coming back:
  - `a_run_measured_as_it_goes_keeps_the_key_of_one_that_is_not`: a cold run under all of those switches, then a
    warm run under none, is kept; a switch that changes outputs (`RERAIL_AST_PARTS=off`) still runs it again.
  - `every_switch_a_profile_reads_is_left_out_of_the_runs_key`: every switch named in the profile's sources
    (`logic/profile`, the `rerail_prof` crate) is `measures_only`, so a new sampler switch is left out without
    anyone remembering to list it. A dump (`RERAIL_DUMP_*`) turns the cutoff off instead (`reuse::wanted`).
