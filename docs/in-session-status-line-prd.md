# PRD: Rerail visibility inside coding-agent sessions

**Document type:** As-built product requirements — describes what we made, not a proposed roadmap.

**Work item:** [NER-1464](https://linear.app/nera/issue/NER-1464/spike-surface-rerail-in-agent-sessions-configurably-and-link-back-to)

**Delivery status:** Implemented, installed locally, and verified in real agent sessions. These changes have **not yet been published as a package release**.

## 1. Executive summary

Rerail now makes its presence visible inside supported coding agents. Developers can see how many rules are configured for their current session, how many times Rerail has successfully delivered guidance, and whether evaluation is running or has failed. They can open the relevant rules or the exact session in Rerail’s local dashboard without searching for it.

OMP and Claude Code show a compact status row, or **HUD**. Codex provides the same session context on request through a `$rerail` skill, rather than pretending to support a persistent footer.

This is part of normal installation, not a separately enabled demo. Presentation preferences live in SQLite. Hiding the HUD does not disable enforcement, and the display does not present guidance deliveries or violations as proof that the agent fixed anything.

## 2. Problem and intended value

Without an in-session view, a developer must leave the coding agent and inspect Rerail separately to answer:

- Is Rerail available for the session I am working in?
- What rule coverage is configured for this session?
- Has Rerail actually delivered guidance, or only observed a possible issue?
- Where can I inspect this session’s evaluation history?

A generic link to a dashboard is insufficient: it can leave the user looking at another session or project. Replacing an existing status line would also remove information the developer already relies on.

**Intended value:** make Rerail easier to understand and inspect without interrupting normal coding-agent use. Reduced context switching and improved user confidence are expected benefits, not measured outcomes from this work.

## 3. Users and goals

**Primary user:** a developer working with Rerail in OMP, Claude Code, or Codex.

**Secondary user:** someone investigating an agent session who needs a direct link to its recorded activity and evaluation history.

Goals:

1. Make current-session coverage and guidance activity visible with minimal visual noise.
2. Provide trustworthy navigation to the right rules and session.
3. Let users hide the status line without weakening rule enforcement.
4. Install, upgrade, and remove the integration without destroying existing user customization.
5. Respect each coding agent’s actual UI and permission capabilities.

## 4. Delivered experience by coding agent

A **coding-agent host**, also called a harness, is the application running the agent.

| Host | Status-line experience | On-demand experience | Important behavior |
| --- | --- | --- | --- |
| OMP | One compact widget below the editor | `/rerail` returns current-session detail and its session link | Uses the standard Rerail extension; does not replace the existing footer |
| Claude Code | Rerail is composed with the existing status line | `/rerail` returns current-session detail; explicit launcher commands remain available | Preserves existing global and project status commands, including settings inherited from a Git worktree’s main checkout |
| Codex | No new persistent footer | Explicit `$rerail` skill returns current-session detail and its session link | Uses native session identity and normal local-execution approval; does not bypass the sandbox |
| Pi and Cursor | No new status-line surface delivered by this work | No new native on-demand journey is claimed here | Existing integration registration is preserved |

### The compact status row

Example:

```text
Rerail | 185 rules | steered 3×
```

Each segment has a specific purpose:

| Segment | Meaning | Destination |
| --- | --- | --- |
| `Rerail` | Entry point to this session’s Rerail context | Exact current session |
| `185 rules` | Effective rule coverage configured for this session | Rules page in the same project database |
| `steered 3×` | Three successful deliveries of guidance to the agent | Exact current session |

Only the `Rerail` label uses Nera’s canonical brand color. Raw URLs do not need to occupy the status line. Link activation depends on the terminal’s hyperlink support.

The row can append a short state such as `checking…`, `evaluation failed`, or an unavailable indicator. Detailed violations stay in the explicit detail view and dashboard.

### What the numbers do—and do not—mean

- **Rules** means configured coverage for the session, not the number of rules matching the current file or the number of checks that passed.
- **Steered** counts successful steers. A bundled guidance response counts as one steer. Observations, matches, queued guidance, and failed sends do not count.
- A successful steer does **not** prove that the model read, followed, or corrected the issue.
- Violations are evaluation output, **not an outstanding-work queue**, a count of required fixes, or a signal that work is ready for human review.
- Unknown history is not displayed as zero. For example, older sessions without complete tracking can show `steering unavailable`.
- Local service health and evaluation success are separate. Rerail can be running normally while a code-rule evaluation reports failures.

## 5. Core user journeys

### A. Install and start working

1. The user installs through the standard Rerail setup path.
2. Setup registers the appropriate integrations for detected hosts, including the HUD or Codex skill.
3. The user starts a new host session so it loads the registration.
4. OMP and Claude show the compact HUD; Codex makes `$rerail` available.

No separate HUD feature flag, prototype launcher, or extra OMP extension is required. Native host trust and approval prompts still apply; installation does not bypass them.

### B. Inspect what Rerail is doing

1. The developer sees configured coverage and delivered-guidance activity.
2. They select the rule count to inspect rules, or select `Rerail` / the steer count to inspect the current session.
3. The dashboard opens with the originating session selected.
4. If the session cannot be resolved, Rerail shows an unavailable state rather than selecting another session.

### C. Work quietly without disabling rules

1. A presentation preference is set to `silent` in the project database through Rerail’s API.
2. The status line disappears on a subsequent refresh.
3. Rule evaluation and guidance delivery remain independent of that display choice.
4. Explicit `/rerail` or `$rerail` requests remain available.
5. Removing the override restores the applicable default.

This work delivers the preference storage and API, **not a dedicated end-user preference settings screen**.

### D. Use Rerail from Codex

1. The user explicitly invokes `$rerail`.
2. The skill requests normal permission to read the local Rerail service outside Codex’s network-isolated execution sandbox.
3. On approval, it returns the current thread’s coverage, guidance total, evaluation summary, and exact session link.
4. If access is denied or unavailable, it reports the restriction. It does not disable the sandbox, change configuration, start a service, or substitute another session.

### E. Upgrade or remove the integration

Repeated setup does not duplicate HUDs or nest status wrappers. Upgrades retain the original Claude status command’s recovery state, including project-specific registrations. Removal restores the previous status behavior while preserving unrelated settings and later user edits.

## 6. Product requirements and acceptance criteria

| ID | Requirement | Acceptance criterion |
| --- | --- | --- |
| P1 | Automatic installation | Standard setup installs the supported HUDs and Codex skill without a separate feature opt-in |
| P2 | Session-specific status | Displayed coverage and guidance totals belong to the originating session; unknown totals are not invented |
| P3 | Correct navigation | Session links validate the host, session, workspace, and project database; missing or mismatched targets do not fall back to another session |
| P4 | Independent presentation | `silent` hides the status line without changing enforcement; explicit detail remains available |
| P5 | Database-owned preferences | Presentation settings are stored in SQLite, with deterministic override precedence and no separate presentation configuration file |
| P6 | Non-destructive composition | Claude retains its existing status output and metadata; project/worktree overrides are handled rather than silently hiding Rerail |
| P7 | Safe lifecycle | Repeated setup is idempotent; upgrade and removal preserve user state and restore owned changes appropriately |
| P8 | Responsive operation | Repository discovery does not block OMP readiness; background discovery does not lose concurrent tool or shell edits |
| P9 | Bounded status reads | The HUD reads a compact evaluation summary instead of downloading full violations on every refresh |
| P10 | Respect host security | Codex uses native hook trust and execution approval, not persisted bypass flags or weakened sandbox settings |

These are delivered requirements. Their verification is summarized below; they are not claims of measured production adoption or business impact.

## 7. Preferences and refresh behavior

Two presentation modes are supported:

- **Compact:** show the status line where the host supports it; this is the default.
- **Silent:** suppress the status line while retaining explicit reads and independent enforcement.

Preference precedence, from most specific to least specific:

1. Repository and host.
2. Repository default.
3. Host default within this database.
4. Database default.
5. Built-in `compact` behavior.

OMP refreshes approximately every five seconds. Claude defaults to a five-second native refresh interval when none exists, but preserves an existing user cadence. Consequently, a preference change may take until the next host refresh to appear.

Presentation preferences live in the project’s SQLite database. Separate SQLite registration records preserve Claude’s previous status commands so installation can be reversed. Host-native settings files necessarily retain integration wiring; they are not a second store for Rerail presentation preferences.

## 8. Boundaries and known limitations

- **Not a new enforcement system.** This work exposes coverage, delivery activity, and evaluation state; it does not redefine which rules run or when they block work.
- **No claim of correction.** Guidance delivery is not verified remediation, model compliance, or human approval.
- **No universal status line.** Persistent Codex and Cursor HUDs are not delivered.
- **Local, project-scoped operation.** The dashboard serves the local service’s configured project database. A session in another database reports unavailability rather than silently retargeting the service. Cross-project aggregation and synchronized settings are not included.
- **Unavailable service limits preference resolution.** Since preferences are read through the local service, an unreachable service can produce an unavailable indicator instead of guaranteeing that a previously stored `silent` preference remains visually silent offline.
- **Historical counts have limits.** Missing tracking history cannot establish a complete steer total. A crash between guidance delivery and recording its receipt can undercount activity.
- **A “live” session label is not a process heartbeat.** Missing session-end information can leave a recorded session labelled live after its process exits.
- **Native security remains in force.** Codex may require hook trust and approval for the local read. Its CLI must be available separately; Rerail does not install coding-agent applications.
- **Startup readiness is not immediate shell reconciliation.** Discovery stays in the background for startup, path-aware edits and opaque-tool starts; a command tool's result waits for a discovery pass, anchored to a filesystem-clock reading taken before the walk, that reports shell writes as uncaptured changes rather than baselining them. Deletions are reported only for sources the baseline already observed. OMP/Pi's direct awaited hook has no local timer; one-shot hook transport retains a 30-second deadline. That cold reconciliation boundary requires measurement against the deadline, not an assumption of bounded latency.
- **No new analytics or performance SLA.** The validation below establishes exercised behavior, not a fleet-wide reliability, latency, or adoption guarantee.

## 9. Verification and release readiness

Recorded before the current refactor (not fresh verification of subsequent changes):

- **380 tests passed across 47 files**, with TypeScript and lint checks passing.
- A packed installer smoke test ran real `npx @neralabs/rerail setup` against a temporary local registry, exercised the installed integration, and verified uninstall restoration.
- `just rerail-dev` updated the live installation. A subsequent installer dry run reported every registration unchanged.
- Real OMP and Claude sessions rendered the installed HUD. Native terminal captures contained all three hyperlink segments and the canonical brand color without feature flags.
- Session links from OMP, Claude, and Codex selected the originating sessions in the bundled dashboard.
- SQLite-only preference changes hid and restored both status-line HUDs. Claude’s predecessor stayed visible and its existing 60-second refresh cadence was preserved.
- Real Codex executed the installed `$rerail` skill successfully after a one-command native approval.
- OMP became ready while repository evaluation continued. A reproducible shell-write race failed before its fix and passed afterward.
- One live status response measured **448 bytes**, replacing the HUD’s previous multi-megabyte history fetch. This is an observed example, not a general latency benchmark.
- Existing code-rule evaluation failures remained visible; they were not suppressed to make the demonstration look successful.

The integration pass must recheck startup deletion safety, inherited Claude status ownership, catalog invalidation and compact-reader cost before extending these evidence claims.

### Distribution status

The implementation supports `npx @neralabs/rerail setup` and its `just rerail` wrapper. The unreleased checkout has been installed with `just rerail-dev`.

**A public package release is still required before ordinary registry installs receive these changes.** The temporary-registry test proves the packaged install path, not that the public package has been updated.

## 10. Suggested post-release product evaluation

The following are proposed evaluation questions, not instrumentation delivered by this work:

- Do users reach a functioning in-session status after setup and host restart without needing support?
- Do session links consistently open the session users intended to inspect?
- Do users understand the distinction between configured rules, delivered guidance, and corrected issues?
- Does the HUD reduce manual dashboard searches without feeling intrusive?
- Do upgrades and removal preserve existing status customizations?

Product can use these questions to assess adoption and decide whether to prioritize a preference settings screen, additional host surfaces, or cross-project navigation. None of those extensions is committed scope in this PRD.

## References

- [Installed behavior, semantics, and recorded evidence](in-session-surfaces.md)
- [Rerail installation documentation](installation.md)
- [Changelog](../CHANGELOG.md)
