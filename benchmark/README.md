# Benchmarks

Development tools that measure Rerail: native query engines against each other, the dashboard's history reads, and
code-rule steer strategies on real coding agents. Nothing here ships, and nothing here runs in CI.

- `query-parity.ts` (`just rerail-query-parity`) runs the same code-rule workloads through two or more native engine
  builds, each in its own process. It reports every section where their findings, failures or coverage differ.
- `query-parity-semantic.ts` names the semantic-resolution workloads (`resolves`, `usage`) that `query-parity.ts` runs.
- `dashboard-history.ts` (`just rerail-dashboard-history`) seeds a throwaway database with a long audit history (`--audits`,
  `--rules`, `--backlog`, `--sessions`), serves the daemon's API over it, and times what the dashboard reads: each history
  read computed and repeated, one poll while a session keeps committing, recording one repository audit, and how long
  `/healthz` waits while dashboard tabs poll (`--tabs`, `--seconds`).
- `steer-strategy.ts` (`just rerail-steer-strategy`) runs a real harness on one task per host × strategy × trial and
  compares the code-rule steer strategies (`message`, `rewind`, `subagent`). See [Steer strategies](#steer-strategies).
- `merge-assist.ts` (`node --import tsx benchmark/merge-assist.ts [--suites capture,transfer,compare,lookup] [--trials N]
  [--compare-trials N] [--concurrency N] [--cases id,id] [--pairs id,id] [--json out.json] [--verbose] [--keep]`)
  evaluates Merge Assist on `fixtures/host/benchmark/merge-assist/cases.json` through `createMergeAssistService` and
  the person's own agent CLI as judge. Each trial gets a fresh temporary Git repository (worktrees `jeremy` and
  `tim`), database and `RERAIL_HOME`, removed afterwards unless `--keep`. **capture** turns each correction session
  into a decision; **transfer** commits each saved code rule on Jeremy's branch and scans Tim's worktree with teammate
  files written independently; **scope** matches each decision's scope against paths it should and should not cover;
  **compare** scans Tim's decision against Jeremy's and reads the verdict from the finding's tier; **lookup** times
  1,000 generated scopes over 10,000 paths, with no model. It prints each metric against its pre-registered threshold
  (per-case rows with `--verbose`). Capture, transfer and compare spend the person's model budget on every judge call;
  results are never committed.
- `merge-assist-context.ts` (`node --import tsx benchmark/merge-assist-context.ts --build-index out.json
  [--distractors N]`, then `--index index.json --variants none,guard-signals,... --split dev|heldout|all
  [--tasks id,id] [--trials N] [--concurrency N] [--distractors N] [--no-control] [--json out.json] [--keep]
  [--verbose] [--model m] [-c key=value]`, `--sweep control.json`, `--list-variants`, `--validate-oracles`) asks
  whether injecting teammates' decisions into
  a real Codex loop, when it touches files they cover, makes its code follow them, and which protocol does so with the
  least chatter. The dataset, `fixtures/host/benchmark/merge-assist/context.json`, reuses cases.json's project and
  names its ten lasting decisions (team or peer, with the hand-written text and scope the oracles grade through), the
  distractor pool and 26 tasks: per split (`dev` for tuning, `heldout` to confirm), nine that tempt one decision each,
  two neutral ones and two where this branch's decision contradicts a teammate's. Nothing hand-written about a real
  decision reaches the agent: `--build-index` runs the product's `draftDecision` with the person's agent CLI judge on
  each decision's capture case (correction, before and after) and keeps the decision text, scope and validated signals
  it produces, leaving out any the judge calls one-off or does not answer; distractors and the conflict tasks'
  teammate decisions get their signals from `deriveSignals`. The index stores each entry's provenance, every judge
  prompt and each decision's embedding (the bundled model), and is never rebuilt over an existing file. Every run reads
  it through `--index`, after an integrity check that refuses an entry with a field the product does not produce or
  an oracle's regex in it, and reports the oracles' code text the product's own output holds. Each run gets a
  temporary repository and `CODEX_HOME` (`auth.json` a symlink to `~/.codex/auth.json`, an empty `config.toml`, and
  `hooks.json` running `merge-assist-context-hook.ts` on PreToolUse and PostToolUse), then `codex exec --json
  --dangerously-bypass-hook-trust --sandbox workspace-write` for at most six minutes. The hook injects the variant's
  decisions as `additionalContext`; variants are presets in the script (`--list-variants`), `none` the control
  (`--no-control` skips it, for smoke runs). `--distractors N` adds N of the index's distractors to every run (the
  same ones, in the same places, every run), as noise a protocol has to filter. The `guard-*` presets orient on
  conflict pairs at a file's first touch and guard each patch: `signals` fires a decision in scope whose signals the
  added lines contain, `embed` one whose embedding is at least `EMBED_THRESHOLD` similar to a 30-line block of them,
  `both` either, at most three per patch. At every patch, the control included, the hook logs each scope-matching
  decision's signal hits and similarity and the lines added; `--sweep control.json` labels a dev control run's patches
  with their task's oracle and prints recall and fires per clean patch per source and threshold, the threshold to
  freeze in `EMBED_THRESHOLD` (the embed presets refuse to run until it is). `upfront-final` is `upfront-all` with a
  conflict's exact line to end the final message with. `area-orient` injects, on the first touch of an area (a file's
  folder cut to two segments: `src/ui`, `src`), every undelivered decision whose scope covers the file or any project
  file in that area, one terse line each, conflict pairs first, at most 12 per injection (an area the cap left out
  keeps its turn); `area-guard` adds the signals guard for what orientation did not deliver. `--replay run.json`
  re-chooses a report's injections from its logs. Deterministic oracles grade `git diff main`, and every oracle is
  first checked against the labelled files in cases.json (`--validate-oracles` alone needs no model). It prints, per
  variant: the tempt-task violation rate with a Wilson interval and its reduction against the control, the guardrails
  (done rate, injected tokens, neutral overreach and violations, chatter to the person, conflict handling), the
  delivery funnel, cost deltas (with the total: Δ input plus Δ output tokens against the control, what the run
  actually consumed) and the hill-climbing score (per-run rows with `--verbose`). Every run spends the person's Codex
  budget.
- `merge-assist-guard.ts` (`node --import tsx benchmark/merge-assist-guard.ts --index index.json [--thresholds ...]
  [--hook-calls N]`) scores the guard offline, with no agent: each labelled teammate file and project file as a
  patch, it prints per real decision and source the recall on its caught files, fires on its passed files and cross
  noise, distractor fires per patch, an embedding threshold sweep on these forms (reported, never frozen from here),
  the hook's model load and embedding time per call, and fails when a teammate file line reached the judge.

The workloads are host test data under `fixtures/host/benchmark/<kind>/`. Benchmark results are never committed. Keep
them out of the repository, and put headline numbers in prose (with corpus commits and tool versions) where a change
needs them. For the engine's own micro-benchmark, see `npm run bench:code-rule-dsl` (`scripts/dsl/`).

## Steer strategies

```sh
npm run build:ts
node --import tsx benchmark/steer-strategy.ts [--hosts pi,omp,codex,cursor,claude-code] \
  [--strategies message,rewind,subagent] [--trials N] [--json out.json] [--timeout-ms N] [--keep] \
  [--compaction-probe] [-- extra host args]
```

Every trial is a live provider session; it spends model budget. Each one copies
`fixtures/host/benchmark/steer-strategy/project` into a fresh macOS Seatbelt sandbox with its own `HOME` and
`RERAIL_HOME`, starts an in-process daemon that the harness reaches through `RERAIL_ENGINE_SOCKET`
(`RERAIL_AUTOSTART=0`), and starts the repository with one steer-mode code rule: an f-string SQL `.execute`
(`SQL_EXECUTE_FSTRING`, as in `scripts/host/host-lifecycle.ts`), scoped to `api/**/*.py`. It stores the strategy in
the checkout's audit settings before the harness starts, and turns the experimental switch on
(`RERAIL_EXPERIMENTAL_STEER_STRATEGIES=1`) for the daemon and the harness, without which every steer is a `message`. The harness then runs `prompt.md` in print mode. Its first
step writes the forbidden call into `api/lookup.py`. The rest asks the agent to implement `api/inventory.py` per
`SPEC.md`, make `python3 -m unittest discover -s tests` pass, and write the README's usage section. So the steer
arrives with work left to do.

Harness commands, models (`RERAIL_PI_MODEL`, `RERAIL_OMP_MODEL`, `RERAIL_CODEX_MODEL`, `RERAIL_CLAUDE_MODEL`),
binaries (`RERAIL_OMP_EXECUTABLE`, `RERAIL_CLAUDE_BIN`, `RERAIL_CURSOR_AGENT_BIN`), credentials and the sandbox come
from `scripts/host/host-harness.ts`, which the host smoke shares. Arguments after `--` go to the harness before the
prompt, for example `-- --provider azure --model <deployment>`.

Before any trial runs, the runner checks two things:

- A harness whose CLI is missing is skipped, with a `skip <host>:` line.
- A harness whose `dist/` entry is older than `src/` stops the run.

A missing credential (`FAILED prerequisite`) skips the rest of that harness's trials.

Metrics per trial, printed as a mean per host × strategy (`--json` also writes every raw trial):

- **task done**: the runner's own check after the harness exits. The fixture's unit tests, copied back over the agent's
  `tests/` so an edited or deleted test cannot pass, must all run (11) and pass, inside the sandbox.
  README's Usage section must have lost its TODO and must show `Inventory(` and `add_item`.
- **steers**, **in result** and **rewound**: the rule's `steer` deliveries in Rerail's ledger
  (`.rerail/sessions/*/events.jsonl`), how many carry a `toolCallId` (they rode their edit's tool result, in OMP and
  Pi, instead of a message), and how many carry `rewound` (the edit was rolled back). The raw JSON also records each
  delivery's `strategy`.
- **violation left**: whether a file under `api/` still matches the rule. This is a line regex, so a multi-line
  f-string goes unseen.
- **subagent calls**: calls to the harness's subagent tool:
  - Pi `rerail_fix` and OMP `task`, read from the harness's session transcript.
  - Codex `spawn_agent`, Claude Code `Task`/`Agent`, Cursor `Task` (`taskToolCall`), read from the JSON stdout. This
    detection is best effort, because these event shapes are not a stable contract.
- **wall s**, **tokens** and **cost**, where the harness reports them:
  - Pi and OMP: assistant usage from the transcript, plus the `details.usage` of each subagent result.
  - Codex: `turn.completed` usage. Codex reports no cost.
  - Claude Code: the `result` event's usage and `total_cost_usd`.
  - Cursor: whatever usage its `result` event carries.
- **split-turn starts rule / note / prompt** (Pi and OMP): the host's compaction cut logic replayed offline over the
  transcript. It runs at every `keepRecentTokens` threshold where the cut moves, and the cell counts the distinct
  split-turn cuts by what their turn starts at: a `rerail.rule` steer message, Rerail's `rerail.session-note`, or the
  user's prompt (anything else is shown as `+N other`). A split-turn compaction summarizes that turn start as the
  "Original Request", so only `prompt` keeps the person's task there.
  - Pi: its exported `findCutPoint` and `estimateTokens`.
  - OMP: `@oh-my-pi/pi-agent-core` is Bun-only, so the runner carries a line-for-line copy of its cut functions
    (`src/compaction/compaction.ts`, cited in the code) with the byte-estimate tokenizer.

  Under `message` and `subagent`, the steer is a custom message and claims the later split turns. Under `rewind`, the
  steer lives in the tool result and claims none. In the first turn of a session the session-start note, which OMP and
  Pi also receive as a custom message right after the prompt, claims every split turn the steer does not, under every
  strategy: the prompt only starts the turn from the second prompt on.

`--compaction-probe` (Pi only) forks each trial's session and compacts it for real in Pi's RPC mode, with Rerail loaded
(its compaction guard included). It compacts at the smallest `keepRecentTokens` whose cut starts the turn at the steer
(else any split turn), set through the sandbox's global Pi settings, which need no project trust. It then asks what the
user asked for. The raw JSON records the summary's "Goal" and "Original Request" (empty once the guard folded a
steer-led turn into the history) and whether the answer names the task, the rule, both or neither. This costs a
compaction and one more turn per trial. The split-turn column above replays the host's cut logic without Rerail's
guard: it measures how often a turn would start at a Rerail message, not what the guarded summary says.

Caveats:

- The compaction analysis uses the cut code installed in `node_modules`. The CLI on `PATH` may be another version;
  the run prints both.
- The analysis reads the branch from its newest compaction onward without Pi's projection. That is exact for a
  session with no compaction or context edit. A trial that auto-compacted is approximate.
- A single trial is an anecdote. Use `--trials` and report means with the harness versions and models.
- `--keep` leaves each sandbox in `/tmp` for inspection. It holds the imported access token, so delete the sandbox
  when you are done.
