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

- **task done**: the runner's own check after the harness exits. The unit tests must pass, run inside the sandbox.
  README's Usage section must have lost its TODO and must show `Inventory(` and `add_item`.
- **steers** and **rewound**: the rule's `steer` deliveries in Rerail's ledger (`.rerail/sessions/*/events.jsonl`),
  and how many of them carry a `toolCallId`, meaning the edit was rolled back. The raw JSON also records each
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

`--compaction-probe` (Pi only) forks each trial's session and compacts it for real in Pi's RPC mode. It compacts at
the smallest `keepRecentTokens` whose cut starts the turn at the steer (else any split turn), set through the
sandbox's global Pi settings, which need no project trust. It then asks what the user asked for. The raw JSON records
the summary's "Original Request" and whether the answer names the task, the rule, both or neither. This costs a
compaction and one more turn per trial.

Caveats:

- The compaction analysis uses the cut code installed in `node_modules`. The CLI on `PATH` may be another version;
  the run prints both.
- The analysis reads the branch from its newest compaction onward without Pi's projection. That is exact for a
  session with no compaction or context edit. A trial that auto-compacted is approximate.
- A single trial is an anecdote. Use `--trials` and report means with the harness versions and models.
- `--keep` leaves each sandbox in `/tmp` for inspection. It holds the imported access token, so delete the sandbox
  when you are done.
