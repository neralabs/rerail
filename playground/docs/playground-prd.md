# PRD: Rerail Rule Playground

## 1. Purpose and authority

The Rerail Rule Playground is a development-only environment for observing how a saved Rerail rule evaluates realistic activity and delivers guidance inside **actual OMP**. It combines live implementation feedback, reusable scripted experiments, and genuine authenticated agent execution without using the original checkout as the execution sandbox.

This PRD records the product contract agreed while the playground was designed: user-facing requirements, not a description of the implementation.

Later decisions override earlier proposals. In particular:

- **Integrated development workflow, not an installed production feature.**
- **Read-only rule UI, not a sandbox rule editor or save-back flow.**
- **Automatic database-driven reruns include paid execution, without an additional opt-in toggle.**
- **Actual saved-rule evaluation, not a synthetic rule or injected success in trigger/real-agent modes.**

This is a product contract, not a claim that every requirement has been independently verified against the implementation. Source references ``–`` are indexed in section 13.

## 2. Problem

The original investigation required deliberately triggering real violations to inspect Rerail guidance in OMP. A static gallery would show captured appearances, but would not update as Rerail implementation changes or explain delivery timing in an interactive session.

The first live playground improved that feedback loop but used a synthetic rule and scripted model responses. It could demonstrate delivery mechanics, not whether a user's saved rule behaved correctly during realistic work. The requested feature extends that live environment to actual rules selected from the Rules list.

### Primary users

- **Rerail developers:** change delivery, rendering, integration, or native evaluation code and inspect the effect in real OMP.
- **Rule authors using an LLM:** change rules in the local database, reuse the same scenario, and inspect how the changed rule behaves.

### Product outcomes

1. Move from **Rules → Playground → Rules** within one development application.
2. Distinguish what a scenario is expected to do from what the actual evaluator and agent did.
3. Iterate on a rule without rewriting its test inputs after every rule edit.
4. Compare deterministic input replay with genuine model behavior.
5. Make execution, recurring account usage, isolation, and unsupported capabilities understandable and controllable.

## 3. Scope and non-goals

### In scope

- A live, interactive OMP terminal, not a web imitation of OMP.
- Integration with this checkout's dev daemon through `just rerail-playground`, with isolated workers started on demand.
- Launching from a saved rule, switching rules, returning to the Rules list, and refreshing rule details from the local daemon/database.
- Both conversation rules and code rules, exercised through their genuine evaluation paths.
- Explicit LLM generation and revision of contrasting structured scenarios.
- Delivery preview, scripted end-to-end trigger, and real-agent execution.
- Persistent checkout-local experiments and attributable results.
- Coalesced database-edit reruns, including paid execution.
- Sandbox file tools and explicitly permitted, bounded shell commands.
- Live feedback on development source changes and native build failures.

### Explicit non-goals

- Shipping the playground worker, generator, playground-only dependencies, or execution endpoints in the normal production build/package, or exposing its launch action in the normal application. Gated shared UI code may remain in production artifacts; zero playground-related UI bytes is not required.
- A static recording gallery as the primary experience, or a requirement to use Storybook itself.
- Rule-editing forms, sandbox rule drafts, threshold/delivery overrides that change a separate draft, or Save back UI.
- A parallel or simulated rule evaluator.
- Automatically changing a rule or repeatedly regenerating examples until results appear successful.
- Implementing genuine OMP time travel, rewinding completed tool writes, or silently substituting a steer for unsupported time travel.
- Broadening this feature to Pi, Claude, Cursor, or Codex playground UIs merely because other coding agents can host Rerail.
- Reopening the earlier installer, database-reset, release-distribution, or general runtime-performance work as playground deliverables.

## 4. Primary journeys

### Journey A — Inspect and exercise a saved rule

1. Start the playground with `just rerail-playground` and open the client URL it prints.
2. Pick a saved rule in the client's rule selector.
3. Inspect the read-only rule and any previously saved experiment.
4. If scenarios are needed, explicitly choose **Generate scenarios**.
5. Inspect the proposed positive and negative cases, including their explanations, inputs, and the runtime activity expected to trigger the selected rule. The positive case must target the rule's real guidance-delivery path, not merely place a violation in a fixture.
6. Choose a scenario and execution mode, then explicitly Run. For an edit-triggered code rule, the positive scripted case performs a relevant sandbox write/edit; a read-only inspection of an already-invalid fixture does not exercise that trigger. For text-triggered rules, use the applicable text/event boundary rather than requiring an unrelated file change.
7. Observe the complete chain in actual OMP: scenario activity → selected-rule evaluation → delivery decision → guidance rendered and, when the configured delivery mode calls for it, present in the model's outgoing context. In Real agent mode, inspect the model's subsequent behavior without treating receipt as proof that it followed the guidance.
8. Compare expected versus observed outcomes at each stage. A startup/log-only violation is diagnostic evidence, not a successful end-to-end trigger. If the intended trigger or delivery did not occur, show the unmet expectation and why; do not substitute a green “match,” force delivery, or change the rule. Intentional `log` (record-only) behavior remains explicit rather than being described as delivered guidance.
9. Press **Back to Rules** to return to the previous filters and selected row.

**Critical implementation gap identified in this journey:** the generated admin-async scenario asked the agent to “Read the implementation without changing it” while the fixture already contained the prohibited thread bridge. The reported `match` came from an initial repository scan in log mode; no edit-triggered evaluation or guidance delivery occurred. That did not fulfill the playground's purpose of triggering the rule and seeing how its guidance works. Correcting the verdict alone is insufficient: positive scenario generation must target the actual triggering activity, and the result must make delivery and model-context receipt independently observable. This is an unmet requirement, not an accepted limitation or evidence of a completed fix.

### Journey B — Iterate through database changes

1. Start an experiment with Run, arming its database watch/rerun loop.
2. Use the user's LLM to edit the selected rule in SQLite through the normal authoring path.
3. The playground refreshes the displayed saved rule.
4. Rapid edits coalesce; the latest version is rerun at most once every 30 seconds using the existing scenario.
5. Compare the new outcome with the pinned inputs and rule snapshot for each completed result.
6. Stop ends the current work and the rerun loop. Further database edits must not restart it.

### Journey C — Investigate an unexpected outcome

1. Run a generated or edited scenario.
2. See whether the observed result matches its stated expectation.
3. Inspect the exact rule snapshot, scenario inputs, fixtures, and trace rather than assuming the rule or scenario is correct.
4. Edit scenario inputs and rerun without another generation request, or explicitly choose **Revise using this result**.
5. Revision uses the prior observed evidence to propose new scenarios; it must not rewrite the rule or conceal the mismatch.

### Journey D — Iterate on Rerail implementation

1. Leave the development playground open with a selected scenario.
2. Change Rerail implementation code and see a fresh replay through the updated implementation where replay is authorized.
3. UI-only changes update the presentation without requiring manual violation creation.
4. Native build failures stop replay and appear in the trace rather than producing results from an apparently updated but stale engine.
5. After repair, the updated implementation can be exercised again. Paid source-change behavior is qualified in section 12; the later paid-rerun approval was explicitly about database edits.

## 5. Navigation, layout, and interaction requirements

| ID    | Requirement                                                                                                                                                                                                                                                                                                  |
| ----- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| UX-01 | Rules exposes **Open in playground** in the development-capable application. The production application has no playground launch action.                                                                                                                                                                     |
| UX-02 | **Back to Rules** preserves the list's filters and selection. Navigation cancels current execution without discarding the saved experiment.                                                                                                                                                                  |
| UX-03 | A prominent saved-rule selector near the top lets the user switch the playground's target rule.                                                                                                                                                                                                              |
| UX-04 | The real interactive OMP terminal remains the primary surface. Users can inspect/expand guidance, scroll, and use supported normal OMP interactions.                                                                                                                                                         |
| UX-05 | The side panel provides **Rule** and **Event trace** tabs. Rule view is read-only and refreshes regularly from SQLite through the daemon.                                                                                                                                                                    |
| UX-06 | Scenario prompts, scripted events/tool calls, fixture files, explanations, and expectations remain inspectable and editable. Editing scenarios is distinct from editing rules.                                                                                                                               |
| UX-07 | Do not display the generation-access disclosure or the database-watch/recurring-usage notice and expandable explanation. Run buttons read **Run** or **Run real agent**, without “& watch DB”. These later user-directed removals supersede the earlier notice requirement; execution behavior is unchanged. |
| UX-08 | Time-travel warnings are contextual, not permanently displayed. If an unsupported time-travel option is presented and selected, show the limitation and prevent unsupported playback; do not simulate rewind.                                                                                                |
| UX-09 | Scripted playback offers restart/run, pause, resume, and step/advance controls. Pausing must actually stop scripted progression.                                                                                                                                                                             |
| UX-10 | Delivery configuration follows the selected saved rule; independent controls must not silently cause execution of a different rule configuration.                                                                                                                                                            |

Sources:.

### Delivery timing must remain observable

The playground must distinguish:

- `log`: record without injecting guidance into the model's conversation.
- `steer`: guidance enters the active conversation through OMP's real steer mechanism.
- `block`: stored and shown as the rule's mode; until block enforcement lands, guidance is delivered exactly like `steer`.

The trace and terminal must not collapse “matched,” “queued/accepted,” and “present in model context” into a single success claim. Step at idle begins another user turn; Resume alone does not begin that turn.

## 6. Rule identity and evaluation requirements

| ID      | Requirement                                                                                                                                                                                     |
| ------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| RULE-01 | Support saved conversation rules and code rules through their actual evaluation paths. Never demonstrate a code rule by substituting a cosine simulation.                                       |
| RULE-02 | SQLite is the rule-authoring source of truth. The user drives rule changes through their LLM; the playground observes and runs them.                                                            |
| RULE-03 | Activate only the selected rule inside the sandbox, even if it is disabled in the saved database. Clearly explain this local activation; do not change the saved enabled state.                 |
| RULE-04 | Every scenario execution starts with fresh rule execution/repeat state. An earlier `repeat: once` result must not suppress the same rule in another scenario.                                   |
| RULE-05 | Capture the exact rule version and scenario inputs for each run. A database refresh must not change the rule underneath an active run or relabel an old result as belonging to the new version. |
| RULE-06 | Never change a rule's reference, threshold, clauses, guidance, or other definition to force a generated scenario to pass.                                                                        |
| RULE-07 | Missing capabilities or structural incompatibility are reported distinctly from a valid no-match result.                                                                                        |

Sources:.

## 7. Scenario generation, editing, and validation

### Generation and revision

| ID     | Requirement                                                                                                                                                                                                                                                                                                                |
| ------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| GEN-01 | Generation is an explicit LLM operation. Merely opening, switching, restoring, reconnecting, or refreshing a rule must not generate scenarios.                                                                                                                                                                             |
| GEN-02 | Generate a contrasting pair: one expected to trigger and one expected not to trigger, each with a short explanation of why. Expectations are predictions until evaluated.                                                                                                                                                  |
| GEN-03 | The generator may inspect the original checkout without file-by-file approval and use relevant repository context. This is unrestricted inspection within the checkout, not permission to mutate it or access the entire host.                                                                                             |
| GEN-04 | Scenarios are structured replay data: task/user prompt, scripted assistant/tool events, fixture paths/content, and expected outcomes. They are not arbitrary generated harness programs.                                                                                                                                   |
| GEN-05 | Users can inspect and edit scenarios, then reuse them without paying for another generation request. Invalid scenario input is surfaced before execution.                                                                                                                                                                  |
| GEN-06 | Validate behavior with the actual selected rule evaluator and show expected versus observed outcomes. Do not silently regenerate, alter expectations, or retry until green.                                                                                                                                                |
| GEN-07 | **Revise using this result** is an explicit new generation action informed by the previous pinned run and its observed evidence.                                                                                                                                                                                           |
| GEN-08 | Generated text-only steps may omit `toolCalls`; normalize omission to an empty list before replay. Preserve declared valid calls and reject malformed supplied lists rather than silently dropping them.                                                                                                                   |
| GEN-09 | Generation is single-flight: repeated Generate/Revise clicks must not start or replace an in-flight paid request. Show a distinct generating state without a transient stopped/running flicker, disable conflicting execution actions, and retain Stop. Controls recover after completion, failure, or cancellation. |
| GEN-10 | Positive scenarios must target the selected rule's actual runtime trigger and configured guidance path. Distinguish fixture/setup violations from violations caused by scenario activity; a pre-existing violation plus read-only inspection is not a positive end-to-end case for an edit-triggered code rule. Negative cases exercise the corresponding boundary without satisfying the rule, rather than merely omitting the activity being tested. Real-agent outcomes remain predictions, not guaranteed triggers. |

Sources:.

A generated candidate is not itself proof of matching. Generation and execution must remain distinguishable in the interface. Generated fixture code must not execute merely because an experiment was opened or generated.

### Meaning of a result

| Result category                | Meaning                                                                                                                                   |
| ------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------- |
| Match                          | Actual evaluation found the selected rule's condition. Identify whether this was a startup/setup diagnostic or scenario-triggered evaluation, and whether it was log-only. A violation alone does not prove that the intended trigger was exercised or guidance delivered. |
| No-match                       | The applicable scenario was genuinely exercised and did not trigger the rule. This can be a valid outcome in scripted or real-agent mode. |
| Cannot exercise / incompatible | The necessary tool, rule capability, or compatible scenario is unavailable. This is not a negative match.                                 |
| Execution error                | Generation, execution, build, or evaluation failed. Failure is not evidence of no-match.                                                  |
| Delivery preview               | Evaluation was deliberately bypassed to inspect delivery. No matching assertion is made.                                                  |

Report detection and delivery separately: triggering activity (or its absence), evaluation origin and mode, violation, delivery decision/queue state, terminal rendering, and actual model-context receipt at the configured boundary. A positive end-to-end expectation is unmet when only a startup/log-only violation exists. Genuine model response is observable only in Real agent mode; scripted responses cannot establish how a reasoning model reacts to guidance. Preserve intentional no-delivery (`log`) behavior and delayed delivery semantics rather than overriding them to make the scenario pass.

Mismatches remain visible and useful. The generator may be wrong, the rule may be too narrow or broad, or the scenario may expose a real defect; the system must not choose a reassuring verdict on the user's behalf.

## 8. Three execution modes

| Mode                            | Inputs and behavior                                                                                                                                                      | What it does not prove                                                                                                |
| ------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------- |
| **Delivery preview**            | Use controlled scenario activity and the selected rule's guidance through the real production delivery path and actual OMP UI. Explicitly bypass evaluation.             | Does not establish that the rule would match or that a reasoning model would follow it.                               |
| **Scripted end-to-end trigger** | Replay saved prompts, assistant events, and declared tool actions against the actual unchanged saved rule, through evaluation, delivery, and rendering.                  | Scripted responses are not genuine model reasoning or proof of compliance with guidance.                              |
| **Real agent**                  | Give an authenticated model the scenario task and sandbox fixtures. Let it make genuine text/tool decisions and evaluate those actions through the actual selected rule. | A trigger is not guaranteed. Do not inject guidance as a fallback or instruct the model to fabricate receipt/success. |

Additional requirements:

- The scripted provider is a deterministic event script, **not a locally running AI model**. Explain this distinction accurately.
- Real-agent execution uses an authenticated model available through OMP, rather than presenting scripted output as real reasoning.
- Only Delivery preview is allowed to bypass rule matching in the final rule-specific design.
- Code rules with LLM clauses require genuine live evaluation when that capability exists; do not substitute a mock or cached verdict and describe it as a new evaluation.
- Unsupported delivery/host/capability combinations must be explicit, without silent downgrades.

## 9. Execution lifecycle and account usage

### Run and automatic reruns

| ID     | Requirement                                                                                                                                                                                                            |
| ------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| RUN-01 | An explicit Run begins execution and arms the selected experiment's database-driven rerun loop. A restored or explicitly stopped experiment remains stopped until Run.                                                 |
| RUN-02 | Refresh the selected rule regularly from the database. While the loop is armed, coalesce edits and start reruns of the latest rule version **at most once every 30 seconds**. Do not queue every intermediate version. |
| RUN-03 | Reuse the existing scenario after rule edits. Never regenerate scenarios as a side effect of automatic reruns.                                                                                                         |
| RUN-04 | Automatic database-driven reruns include Real agent and other paid evaluation. **No additional paid-auto-rerun toggle is required.** Make recurring account usage visible without interrupting the agreed workflow.    |
| RUN-05 | A changed rule gets a new immutable run snapshot; do not mutate the rule configuration inside an already-active execution.                                                                                             |
| RUN-06 | If an edit makes the scenario incompatible, explain that revision is needed rather than producing a misleading comparison.                                                                                             |
| RUN-07 | Stop cancels active work and the rerun loop. Back, rule switching, and disconnect likewise cancel the current experiment's execution and revoke its running-loop authorization.                                        |
| RUN-08 | Opening, switching, reconnecting, or restoring does not itself authorize model execution. Paid automatic database reruns are authorized by the armed Run loop, not by mere UI presence.                                |
| RUN-09 | Real-agent runs are bounded; the accepted real-agent checkpoint specifies a three-minute lifetime. Expiry and cancellation clean up owned workers and temporary authentication state.                                  |

Sources:.

The 30-second limit concerns rerun starts after rule edits, not a requirement to spend usage every 30 seconds when nothing has changed. No exact database polling interval was agreed. The scope of paid authorization on implementation-source rebuilds remains qualified in section 12.

### Lifecycle summary

| Event                            | Required outcome                                                                                                                     |
| -------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------ |
| Open/restore an experiment       | Restore selection, scenarios, and latest completed result; do not execute.                                                           |
| Generate/revise                  | Explicit model operation producing inspectable candidates, not an invented observed verdict.                                         |
| Run                              | Execute the selected scenario and arm database-driven reruns.                                                                        |
| Database edits while armed       | Refresh details; coalesce to the latest snapshot; rerun the same scenario within the 30-second rate limit, including paid execution. |
| Database edits while stopped     | Refresh the displayed rule without starting execution.                                                                               |
| Incompatible rule/scenario       | Explain inability to exercise; preserve useful previous evidence.                                                                    |
| Stop/Back/rule switch/disconnect | Cancel current work and the loop; retain saved experiment state.                                                                     |

## 10. Persistence, isolation, and runtime boundaries

### Persistence

- Store experiments in checkout-local development storage, separate from the authoritative production rule records.
- Preserve scenarios, experiment selection/state, and the latest completed result across navigation and restarts.
- Each result retains the rule and scenario snapshots it actually exercised, including its observed outcome and trace.
- Do not erase or misattribute previous completed evidence merely because the selected rule has since changed.
- Restoring state never restores execution authorization or starts paid work.

The conversation did not agree to unlimited run history, shared/team experiment storage, cloud synchronization, or a save-back workflow.

### Sandbox and command permissions

- Generation and execution leave the original checkout read-only. Fixtures and replay changes belong to the disposable sandbox.
- Sandbox file reads, searches, writes, and edits are available by default.
- Shell execution requires explicit **Allow commands** permission. Scripted commands must be visible before execution.
- Commands are bounded and isolated from the original checkout and authentication files; command network access is denied by default.
- Missing command permission or another required capability produces **Cannot exercise**, not no-match.
- Real-agent authentication uses the existing OMP login through temporary access-only state, preserving the original credential store.
- Cancellation, expiry, browser disconnection, and runtime shutdown must reclaim all owned processes and temporary credential/sandbox state. Cleanup includes command descendants, including children that use Bash job control—not only the initial shell.
- Normal Rerail sessions, the normal database, and unrelated user files must not become disposable experiment state.

### Development-only integration

- `just rerail-playground` brings up this checkout's dev daemon through `rerail-dev` (a host process with a checkout-local home and port), then a host supervisor that serves the client, rule list, OpenAPI document, and control socket from one loopback origin and starts isolated workers on demand.
- Installed Rerail defaults to **http://127.0.0.1:4737**. Each checkout's dev daemon records its own port, so several checkouts run concurrently; a playground reaches only its own checkout's daemon.
- Keep the daemon, API, MCP, database, and isolated-worker boundaries unchanged. The daemon has no playground routes; the supervisor reads saved rules from it over HTTP.
- Unexpected development failure must be explicit, not silently replaced by a different UI. A daemon remains the exclusive writer for its database; the playground never stops or replaces another checkout's daemon or the installed Rerail.
- Worker/harness code, generation machinery, and playground-only dependencies stay outside the production build/package.
- The production application exposes neither playground execution endpoints nor the launch action.
- Gated shared UI code is acceptable in production artifacts. Only generic, dev-gated routing support belongs in the daemon; playground execution machinery remains excluded.
- Live development feedback uses the actual Rerail/OMP paths. A stale native build or unsupported feature must not be masked with a simulated successful result.

## 11. Acceptance criteria

These are behavioral acceptance scenarios, not claims based on inspecting code.

| ID    | Acceptance scenario                                                                                                                                                                                                                                                                                     |
| ----- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| AC-01 | From the development Rules list, open a selected rule in the playground, switch rules from the prominent selector, and return to the original list with its filters and selection intact.                                                                                                               |
| AC-02 | Observe actual OMP terminal interaction and the Rule/Event trace tabs. Rule details refresh after database edits without exposing rule-editing or Save back controls.                                                                                                                                   |
| AC-03 | Generate and inspect a contrasting scenario pair for a conversation rule and a code rule. Execute both cases through real evaluation; distinguish expected labels, actual violations, no-match, and mismatches.                                                                                        |
| AC-04 | Generate a pair containing text-only steps with omitted tool lists: it remains usable. Malformed supplied tool lists are rejected, and valid declared calls are preserved.                                                                                                                              |
| AC-05 | Delivery preview visibly identifies its detector bypass. Scripted trigger and Real agent use the unchanged saved rule; neither manufactures a trigger or injects a fallback success.                                                                                                                    |
| AC-06 | Test a saved disabled rule without enabling it in the database. Only that rule runs in the sandbox; independent scenarios have fresh repeat state.                                                                                                                                                      |
| AC-07 | Observe delivery timing, including `log` guidance staying absent from model context. The trace separates matching, delivery acceptance, and actual model-context receipt.                                                                                                                             |
| AC-08 | Edit scenario inputs without regenerating. After an unexpected result, explicitly revise using its evidence; no automatic retry or rule modification hides the mismatch.                                                                                                                                |
| AC-09 | While Run is armed, make several database edits. Reruns use the latest version, reuse the scenario, and start no more frequently than every 30 seconds. The same behavior includes paid execution without an extra toggle.                                                                              |
| AC-10 | Stop, navigate Back, switch rules, or disconnect during active work. Execution and automatic reruns stop; owned workers, command children, and temporary authentication state are reclaimed. Subsequent edits do not restart a stopped experiment.                                                      |
| AC-11 | Return after navigation or restart. Restore scenarios, selection, and the latest pinned result, with no automatic generation or execution.                                                                                                                                                              |
| AC-12 | Exercise sandbox file operations and explicitly permitted commands. Original-checkout writes, command credential access, and command network access remain prohibited; absent capabilities are not reported as no-match.                                                                                |
| AC-13 | Change development UI and Rerail/native source. Observe the live feedback path and a genuine build-failure state; do not show stale execution as a successful updated run.                                                                                                                               |
| AC-14 | Inspect the normal production application and package: no exposed playground launch action or execution endpoints, and no worker/generator or playground-only dependencies. Gated shared UI code is allowed in the artifacts.                                                                           |
| AC-15 | The removed generation-access and database-watch disclosures are absent. Run buttons omit “& watch DB”. Unsupported time-travel messaging appears only when relevant, with no simulated execution.                                                                                                      |
| AC-16 | Installed Rerail defaults to `http://127.0.0.1:4737`. Run `just rerail-playground` in two checkouts concurrently: each dev daemon and supervisor uses its own loopback ports, each playground lists only its own checkout's rules, and stopping one leaves the other running. Startup waits for daemon readiness, Ctrl-C stops the workers and leaves the dev daemon serving, and unexpected failure is explicit. |
| AC-17 | Start generation, then attempt Generate/Revise again: one request remains active, the UI stays generating, and Stop cancels it and restores the controls. Remove the “Generation access and command isolation” disclosure; the underlying sandbox restrictions remain unchanged.                  |
| AC-18 | Journey A: run the read-only admin-helper scenario against a fixture that already contains the forbidden thread bridge. Identify the startup/log-only violation, absence of scenario edits, and absence of delivered guidance; do not report end-to-end success. Generate a positive scripted case that instead performs a relevant sandbox edit, then verify evaluation of that activity, configured delivery, actual OMP rendering, and model-context receipt at the correct boundary. Its negative counterpart exercises the same edit boundary without introducing the violation. For a text-triggered rule, exercise its actual text boundary without requiring a file edit. Keep the saved rule unchanged; expose unmet expectations in Real agent mode without injecting fallback guidance or presenting scripted behavior as genuine model response. |

**Verification surface:** use a real browser for the playground's visual and interaction acceptance. A screenshot or attractive notification alone is insufficient evidence of model-context delivery.

## 12. Superseded decisions, known limitations, and unresolved details

### Superseded decisions

| Earlier direction                                           | Final agreed direction                                                                                          |
| ----------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------- |
| Static delivery gallery and stored recordings               | Live interactive playground with implementation-change feedback.                                                |
| Separate development launcher                               | `just rerail-playground` brings up the dev daemon itself, with isolated workers on demand.                       |
| Make playground a normal installed Rerail capability         | Development-only; exclude execution machinery and launch surface from production.                               |
| Synthetic rule constructed to match the scripted sample     | Select an actual saved conversation/code rule and evaluate it unchanged.                                            |
| Real agent injects controlled guidance to prove delivery    | Real agent uses genuine selected-rule detection; only Preview bypasses evaluation.                              |
| Editable sandbox rule copy with save-back/conflict handling | LLM authors the SQLite rule; UI is read-only and regularly refreshed.                                           |
| Generator sees only explicitly supplied files               | Generator may inspect the checkout without file-by-file approval.                                               |
| Paid database reruns remain manual, or need an extra toggle | Run authorizes the database-watch loop, including paid reruns, with the 30-second limit and visible disclosure. |

### Disclosed capability limits

- **Native LLM code-rule clauses:** the current native runtime lacks this capability. The playground must show **Cannot exercise**, not a cached, mocked, or negative verdict. The requirement for genuine evaluation is not permission to build a second evaluator in the playground.
- **Time travel:** actual rewind/native TTSR integration was not delivered or agreed as part of this playground contract. Keep unsupported behavior explicit.
- **Platform:** the accepted real-agent checkpoint uses a macOS write sandbox. No cross-platform playground execution commitment was made; do not describe support as universal.

### Details not fully settled

1. **Paid execution on implementation-source changes.** The earlier real-agent checkpoint required source-save cancellation and explicit Run again. The later override explicitly authorizes paid reruns after selected-rule database edits. Nothing re-authorizes paid runs caused by Rerail source rebuilds; do not silently expand the database-edit permission to additional triggers.
2. **Exact presentation and operational limits beyond those named above.** No exact database polling period, command output-size limit, mandatory editor widget, or model-picker policy was agreed. Those should not be presented as user-approved product requirements merely because an implementation chose values.
