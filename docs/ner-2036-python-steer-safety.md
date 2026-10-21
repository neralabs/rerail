# Python steer safety review

This change refines nine Python steers on top of the merged guidance rewrite in PR #162. It preserves the concise style while correcting operator guidance and making behavior-preserving repairs explicit. Every resulting message is at most 140 characters.

| Rule | Problem with the previous guidance | Updated guidance |
|---|---|---|
| E711 | Recommends `is None` even for `!= None` | Separate equality and inequality clauses recommend `is None` and `is not None` respectively |
| F632 | Recommends `==` even for `is not` | Separate operator clauses recommend `==` and `!=` respectively |
| F541 | Removing only the `f` prefix changes doubled-brace runtime strings | Explicitly unescape doubled braces while preserving the runtime value |
| B905 | Reports a missing keyword without explaining intentional truncation | Offer both `strict=True` and `strict=False` |
| F707 | Reports a misplaced handler without saying how to fix it | Move the bare handler after all typed handlers |
| SIM300 | Swapping ordered operands without reversing the operator changes the condition | Reverse ordered operators and preserve equality operators |
| SIM112 | Uppercasing an externally defined key changes POSIX environment lookups | Preserve intentional lowercase deployment contracts |
| ASYNC210 | Switching clients can break an existing HTTP transport interface | Offer a thread helper for request and response handling, preserving the API and options |
| ASYNC222 | A PID-based wait does not imply an async process object exists | Offload the existing wait, preserving arguments and status; await a process object only when it is async |

For example, `value != None` must become `value is not None`, not `value is None`. Likewise, `value is not 2` must retain the negative comparison as `value != 2`. F541 must turn `f'{{ready}}'` into `'{ready}'`, not `'{{ready}}'`.

## Scope

The two comparison rules use separate clauses because the emitted message depends on the operator. Their existing expression matching, suppression and annotation behavior is covered by regression and conformance checks. The duplication in their JSON is the existing matcher partitioned by operator.

Rule scopes and modes remain unchanged. B101 test scope is handled separately in [PR #137](https://github.com/neralabs/rerail/pull/137). Advisory-to-Log proposals and the broader security-rule revisions are outside this change. This is a focused part of [NER-2036](https://linear.app/nera/issue/NER-2036), not a claim that every issue acceptance item is complete.

## Why these safeguards

A prior 48-trial comparison of the longer local drafts and Jeff's merged messages used Haiku 5.5 medium and GPT-6 Luna medium with code-only feedback. One Luna template run under the previous F541 message removed the prefixes but retained doubled braces, failing five of eight hidden checks. The longer local template messages passed all checks. That is evidence for retaining the brace safeguard, not proof that longer messages generally perform better.

The other prior comparison cases mostly passed with either version. There was no general efficiency advantage against Jeff's rewrite. These changes therefore make narrow correctness and compatibility claims rather than promising higher overall task success.

## Automated validation

The new regression suite initially failed 22 tests on the main-branch rule snapshots while 14 existing-behavior cases passed. Each rule update then passed its focused test.

- 93 tests pass across the four focused regression files and the pack catalog suite.
- Full Python fixture conformance passes, including the operator-specific rule changes.
- `npm run check` passes.

The regressions cover reversed and parenthesized operands, mixed-polarity chains, F632 string annotations, unchanged suppression/non-findings, and the safety conditions in the emitted native guidance.

## Final message smoke test

The actual shortened messages were tested against main at `453844720` using Bedrock Runtime ConverseStream in `us-east-1`: Haiku 5.5 medium and GPT-6 Luna medium, four functional tasks, one run per model/condition. Each run received the native finding once after reading the affected code. Task prompts contained functional requirements only; hidden checks stayed outside the model workspace. This is controlled feedback exposure, not a measurement of production rerail delivery frequency.

Both conditions passed all 64 hidden checks on their eight saved implementations. All eight updated-message runs completed normally; seven baseline runs completed normally, while Luna's environment run encountered a Bedrock tool-input HTTP 400 after producing a correct implementation. That invalid execution is not counted as a completed success.

| Task / rule | Model | Before hidden checks | After hidden checks | Before → after remaining findings | Did the update help? |
|---|---|---|---|---|---|

| CSV / B905 | Haiku 5.5 | 8/8 | 8/8 | 0 → 1 | No; both correct, updated run left the finding open |
| CSV / B905 | GPT-6 Luna | 8/8 | 8/8 | 0 → 1 | No; both correct, updated run left the finding open |
| Downloads / ASYNC210 | Haiku 5.5 | 8/8 | 8/8 | 0 → 0 | No functional difference observed |
| Downloads / ASYNC210 | GPT-6 Luna | 8/8 | 8/8 | 0 → 0 | No functional difference observed |
| Templates / F541 | Haiku 5.5 | 8/8 | 8/8 | 0 → 0 | No difference observed in this smoke run |
| Templates / F541 | GPT-6 Luna | 8/8 | 8/8 | 0 → 0 | No difference observed in this smoke run |
| Environment / SIM112 | Haiku 5.5 | 8/8 | 8/8 | 1 → 1 | No functional difference; intentional lowercase key preserved |
| Environment / SIM112 | GPT-6 Luna | 8/8 (execution invalid) | 8/8 | 1 → 1 | No functional difference; intentional lowercase key preserved |

Finding closure fell from 6/8 to 4/8: both updated B905 runs kept implicit zip truncation instead of adding `strict=False`. The SIM112 finding remained in both conditions because the task requires the lowercase deployment key. Functional success and finding closure are therefore reported separately. Requests were 74 before versus 72 after; median model time was 32.6s before versus 35.7s after. This sample does not establish an efficiency improvement.

Only four of the nine rules received this final live-model smoke test. All nine have native regression coverage; the other five do not have new live-model efficacy evidence in this PR. One sample per cell cannot establish comparative superiority or a reliable regression rate. The final smoke supports compatibility, while the operator-specific messages address deterministic guidance errors.

The local experiment is `pr-v7`, with manifest SHA-256 `9cbc91c54e35f20b4aceacd375a930a91fa1caec9be14bec4ad764a4a8d46dd3`. Its snapshot/exposure audit reported zero issues. Raw local experiment artifacts are not included in this source change.
