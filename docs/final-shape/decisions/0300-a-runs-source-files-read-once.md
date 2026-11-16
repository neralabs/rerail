# 0300. A run's source files are read once: the digests' read is held for phase 1's and the run's builders

Status: decided (2026-11-16). Builds on 0104 (phase 1, the waves read no store), 0105 (the observer), 0083
(phases). Found tracing the kernel time of a cold run over PostHog (0296's follow-up).

## Intro

A cold run over all of PostHog (29,885 Python files, 278 MB) spent 98-101 s of kernel CPU of about 300 s, and its
profile charged 25-27 s of CPU to `read`, about 0.5 ms a file. Sampling the whole run on the M3 Max put a third of
the kernel time in `open`: each source file was opened three times.

1. Before phase 1, the observer's digest of each file (`world::bytes_digests`), its bytes then dropped.
2. Phase 1's builder, which read each file it built again (its memo lookup reads and hashes "whatever the index
   said").
3. The run's own builder: its memo lookup before the first wave (whose texts it drops, `keep: false`), then each
   file's wave.

On their own, these calls are cheap (17 us a file on one thread). On the pool, all at once, they are not: on the M3
Max, 16 threads opening and reading the corpus took more wall time than one thread (0.87 s against 0.63 s), for 2.5
times its kernel CPU. Inside the run, where they met phase 1's `realpath`s, an open took about 0.4 ms. Paging was
not the cause: a run with no purge and no decay faulted 8 times fewer pages for the same kernel time.

## Decision

The digest's read is the run's one read of each source file. The run keeps what it read (`corpus::texts::Sources`,
by path, with the stat the observer took before the read) and hands it to phase 1's run (`Texts::sources`, through
`run_read_handing`): passed down the run's flow, held by the run and gone with it, never a process-wide map. A
builder asks for a file's text before it reads it (`Given::text`): phase 1's borrows a copy of what it was handed,
and the run's own takes it, its last reader. A text is used only while its file's stat is the one it was read at;
a file changed since is read again, as before. Bytes that are not UTF-8 are not kept, so the builder still reads and
refuses them. What no builder took is dropped as the run's waves end, before the cross part (`Sources::release`,
which the profile reports).

The texts are held from the digests to the waves: on PostHog about 300 MB more during phase 1 (1.1 GB against 0.74
GB), none at the run's peak (the cross part's), where none is left untaken.

`texts::tests::a_text_is_borrowed_then_taken_and_read_again_once_its_file_changed` checks the borrow, the take, a
changed file and a holder's end.

## Measured

All of PostHog on the M3 Max (shared with other builds, load 14-23), the same 29,747 findings (fa0efeaed6ff):

| | before | after |
|---|---|---|
| `read`, CPU (profiled) | 25-27 s | 0.65-0.71 s |
| kernel CPU, profiled | 98-101 s | 74-80 s |
| kernel CPU, unprofiled | 90 s | 73 s |
| cold wall, unprofiled | 31.0-33.6 s | 29.3 s, 31.1 s |
| warm wall | 1.08 s | 1.05 s |
| peak footprint, unprofiled | 2.53-2.62 GB | 2.62 GB |

## Open

- Each corpus file still gets a `realpath` on a cold store (`Observer::canonical_all`, 8 thread-seconds of
  `getattrlist` in the whole run's sample): a directory's canonical form and the file's name would do where the file
  is no symlink, but macOS's `realpath` also gives a name's case on disk, which that would have to keep.
- The rest of the kernel time: spin-yields (about 20 thread-seconds: the plan model's `RwLock`, rayon's idle spin,
  mutexes) and the store's `pwrite`, `pread` and `fsync` (16).
