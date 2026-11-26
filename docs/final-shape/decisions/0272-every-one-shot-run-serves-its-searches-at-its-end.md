# 0272. Every one-shot run serves its waiting join searches at its end

Status: built (audited 2026-11-23). Decided (2026-11-15). Closes O167. Amends decision 0243 (`drain_at_end`).

## Intro

A slow or heavy line asks for a wider search of its orders (0203 §5). The search runs at idle as the `join-search`
upkeep job, and walks the orders over the relations the request holds (0243).

A command of the CLI ends with its run, before any idle time. Its requests, and the relations they hold, ended
with it. Decision 0243 served them at the run's end within 5% of the process's CPU, but only for `run-policies`.
So `logic`, `run-rr` and the other one-shot commands dropped their searches at exit. Only the note's `wants_search`
survived, and the next run asked again for each line whose work passed `slow_rows`.

## Decision

Every CLI command serves its waiting searches at its run's end (`tools::cli` sets `drain_at_end`). This runs
before the memo is written (`choices::Durable::drop`), so the proposals are written with it. The searches are
served most important first, within `DRAIN_SHARE` (5%) of the process's CPU so far. Each search stops at the
budget too (0245).

A long-lived caller of `cli_in` (the reactor) does not set it, and leaves the searches to the idle job.

The searches the budget leaves are lost with the process, as before. Their lines keep `wants_search` in the
store, and the next run asks again for each line that is still heavy. Keeping the requests themselves in the store
was the alternative. It would need the model rebuilt from stored statistics, and the walks' relations kept, and it
buys nothing while the budget serves every request the queue keeps.

## Measured

Cold, strict, 50 files each, with the join log:

| | searches served at the end | CPU | waiting after |
|---|---|---|---|
| python pack (317 rules) | 64 | 21 ms, of a 921 ms budget | 0 |
| data-layer candidates (75) | 2 + 64 (two runs) | 0.4 + 17.5 ms | 0 |

64 is the queue's cap (`REQUESTS_MAX`): it keeps the most important requests and drops the rest.

`joins::tests::a_one_shot_run_serves_its_waiting_searches_at_its_end` covers both cases: a request served at the
end, and one left to the idle job without the flag.
