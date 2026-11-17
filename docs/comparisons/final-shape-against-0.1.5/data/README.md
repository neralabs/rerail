# The measured figures

One file per area: `build.json` (builds, test suites and trial merges), `linux.json` (the engine and the product on
Linux x64), `windows.json` (the same on Windows 11). A file appears once its area has a figure. Each holds:

| Field | Holds |
|---|---|
| `area` | The area's name. |
| `machines[]` | `id`, `what` (processor, system), `cores`, `memory`, and `role` where given. A run names its machine by `id`. |
| `runs[]` | One measure of one version. Fields below. |
| `findings[]` | `title`, `what` (what a person or maintainer sees), `evidence` (run ids), `severity`. |
| `not_measured[]` | `what`, `why`, `would_take`: a figure we could not take, never estimated. |
| `notes[]` | `what`, `text`: facts that qualify the runs. |

A run:

| Field | Holds |
|---|---|
| `id` | Unique within this folder; the README cites runs by it. |
| `subject` | `N` the new version, `P` 0.1.5, `B` main + six fixes (README.md, "The three versions"). |
| `commit` | The commit measured. |
| `system`, `machine` | `Linux x64`, `Windows 11` or `any (git)`; the machine's `id`. |
| `what`, `state`, `metric`, `unit` | What was measured, in which state (cold, warm, ...), the quantity and its unit. |
| `n`, `median`, `worst`, `values` | Repetitions, their median, their worst, and every value. |
| `series` | Where present: `[[seconds, value], ...]` for a measure over time. |
| `files` | For a trial merge: the files in conflict. |
| `command`, `at` | The command that made the run (local folders shown as `<dir>`) and when it ran (UTC). |
