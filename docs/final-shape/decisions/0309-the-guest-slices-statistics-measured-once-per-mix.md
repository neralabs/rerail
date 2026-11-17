# 0309. The guest slice's statistics measured once per mix, as the run's are

Status: decided (2026-11-16). Found by reading a cold PostHog run's profile for its wall: the chunk phase keeps about
10.5 cores busy, so it is bound by its work, and that work's largest span with nothing inside it was the guest
slice's statistics.

## Intro

A chunk with guests (decision 0055) runs the embed slice over its batch, to find the guests its files ask for. The
slice is planned with statistics of the relations it reads, and each chunk measured them again
(`sampled_statistics`, a sample sorted per column). The run's own statistics are measured once per mix of documents
(`chunk_mix`) and relation, then scaled to each chunk's rows (`measured_once`, `scaled_statistics`). Over PostHog the
slice's measurements were 10.45 s of CPU in 1,896 chunks: 4.5% of the run's CPU, in a span with nothing inside it.

## Decision

The slice's statistics take the run's path: measured the first time a mix reads a relation, kept under their own
key (`guest slice <mix>`, so the slice's numbers stay its own), and scaled to each later chunk's rows by
`estimated_len`, which makes nothing.

## Measured

PostHog on the M3 Max, same 29,747 findings (fa0efeaed6ff):

| | before | after |
|---|---|---|
| guest statistics | 10.45 s | 0.04 s |
| guests, all | 23.38 s | 13.11 s |
| own CPU of every span | 235.8 s | 222.3 s |
| joins in the chunks | 67.2 s | 67.6 s |

Nothing moved elsewhere: the other spans kept their CPU. Cold wall unprofiled: 27.1 s and 35.6 s under load 13–15
from other work (before: 29.3–31.1 s and 36.9–38 s under like loads). The machine's load decides which.
