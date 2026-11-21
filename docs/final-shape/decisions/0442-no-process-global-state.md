# 0442. No process-global state: the run, the connection, and a shared object the caller passes

Status: accepted (2026-11-19). The user: "we want the engine to be re-entrant, multi-thread possible and to not have
global state"; "How can they stay global when multiple runs could be happening at once?"; and on what is shared by
design: "things that are shared by design need to come from an explicit "shared by design" object that is passed.
Otherwise we can't maintain security boundaries."

## Intro

The engine kept much of its state in statics: 563 non-constant statics in its source (test files left out), beside
its thread-locals. Two runs at once in one process collide in many of them: one tape for the native extension,
results by name, phase 1's batch, testing mode's store, a store's stored tables dropped by another run's start,
measurement sums. Settings read once a process go stale when a caller changes the environment between commands.
And what is shared on purpose (a store's writer, a learned cost model, the executors, the interners) was reachable
by any caller in the process, so two callers of different trust could not be kept apart.

## Decision

1. **The test is two runs at once, and two callers of different trust**, not whether a thing is the process's by
   nature.
2. **Where state lives:**
   - **The run** (`logic::run::Run`, decision 0441): a run's results, batches, settings (read as it starts) and
     measurements. Made by the outermost run's scope, carried to its jobs, returned in its record.
   - **The connection**: a connection's own state (components, derived and input relations, temporary tables),
     on the connection's object, not in a process map keyed by connection.
   - **The shared object** (`Shared`): what is shared by design: each store's resources (its writer, handles,
     atoms, frames, observers, journal, symbols), the learned models and their queues, the executors, the
     interners, the caches 0104 has not yet moved into facts. A caller makes it and passes it in (`tools::cli_in`,
     the Node addon, the reactor, a test). A run takes it at its scope and reaches it through the run, never through
     a static; a run with none is refused, so there is no implicit process default. Two callers with two shared
     objects reach nothing of each other's.
3. **What may stay static:** constants; code a crate above registers at load (a function pointer, a device); a
   value computed once from the engine's own files (a regex of a literal, core.rr's declarations). A thread's own
   scratch stays a thread-local (`run_state::STATE`, class Thread).
4. **The guard.** `scripts/check/check_no_global_state.py` (pre-commit) lists every non-constant static in the
   engine's source against `scripts/check/global-state.txt`, each by its class: `stays`, `run`, `connection`,
   `shared`, or `review` (not yet classified by hand). A static not listed is refused; one gone must be cut. Every
   class but `stays` only shrinks.

## Order

1. Settings, logs and measurement sums into the run (`run::Local`).
2. The run's results and batches (the native extension, results by name, phase 1's batch, testing mode's store,
   stored tables, frame counts).
3. The profile into the run.
4. Per-connection state onto the connection.
5. `Shared`: made by the caller, passed through every entry point, carried by the run; the store's resources,
   models, executors, interners and caches into it.
6. The `review` entries, classified by hand and moved.

## Open

- The baseline (2026-11-19): 563 statics: 78 stay, 109 run, 9 connection, 41 shared, 326 review.
- Thread-locals of class Carried were the run's context in thread-locals. Done (2026-11-19): every one is a binding
  in the run's context, as follows. Moved: the profile's switch, directory and stage; the planner's kept plans, plan
  site and four switches; the corpus run's count, token and testing store; the switches' depth; the session and its
  full flag; the drain and whole-relations switches; the placement's binding and recording; the inputs' options;
  the durable side of the kept choices; the phases under way, what they were handed, phase 1's result, the batch and
  the reading; a test's handed store; the run's observations; the derived memo; the chunk; phase 0's flag; the SQL
  ledger; the kept splits; the writes' waves.
  - **What they are.** Of the 40, 37 are a value for an extent of a run: a guard sets it and puts the old one back
    (how deep the run is nested, the phase under way, the batch, the kept plans, the device binding, the profile's
    switch). Two jobs of one run, in different extents, see different values. So they are not fields of `Run`: a
    field is the same for every job of the run.
  - **The run's context** (`rerail_context::bind`, `bound`, `with_bound`): one chain of values by type, on the run's
    thread beside `RUN`. A guard binds a value for its extent (a node on the chain, popped as it drops); a read
    takes the nearest binding, else the type's default (a setting, for the ones the environment sets).
  - **Carried with the run, as one.** `job_run` is the run and this thread's chain; `enter` puts both on the job's
    thread. So every carry of the run carries the whole context: `run_state`'s, `profile::carried`'s, the stream's
    queue, `exec`'s launches, a test's thread (`caller`). Before, a pool's parallel item (`profile::carried`) and
    `exec`'s launches carried the run alone: an item read its pool thread's own device binding, store, waves and
    chunk.
  - **Not context:** a job's own buffers of the kept plans and observations (`OWN_PLANS`, `OWN_PLANNED`,
    `OWN_OBSERVED`), and a test's own store (`OWN`), merged into the run's as the job ends: a job's state on its
    thread, the only parts `run_state` still carries beside the run. `RUN` and `CONTEXT` themselves stay, holding the
    run and the chain. `run_state`'s copied and cloned parts are gone with the thread-locals they carried.
  - **Guards drop in order.** A binding ends as its guard drops; a guard dropped out of the reverse of its order
    would end an inner binding with it. A test build checks it at each drop (a panic names the decision); a struct
    holding two guards declares them inner first (fields drop in order), and a tuple of two is never used. A choice
    made for the rest of the caller's extent, with no guard (`bind_here`: a setter like `profile::set`), replaces
    the nearest such choice of its type and never a guard's binding.
  - **Still kept maps.** A context value holding more than a number or a flag is counted by decision 0104's scan
    (`facts_only::maps`) as the thread-local was: by its type's name, in the module that binds it.
  - **Its cost.** A read walks the chain to the nearest binding of its type. B-now (corpus0, 317 rules, profiling
    build), on one M1 Max at load 17-40, two runs each: 985.5 and 979.3 G instructions before the context
    (65d1d55f07), 985.0 and 1015.9 G with every value moved (420f98435c); the same 463 findings. The two builds'
    runs overlap, and two runs of one build differ by 3%: no cost shows at that resolution.
- Step 3 (the profile), as done:
  - A run's records are the run's (`profile::Records`, a run local). Each thread's shard of them is listed in the run
    it records for. Outside a run nothing keeps them: a test that reads a profile holds a run (`run::scope`, or
    `profile::test_run` in the crates below `logic`).
  - The run thread's phases are the run's. The command's clocks are the command's thread's (`COMMAND`, Outermost).
  - Whether to profile, and where to write the report, are the caller's: bindings in the run's context (`On`,
    `Dir`), which its runs' jobs read too. `profile::carried` carries the run and its context into a pool thread's
    item.
  - The process lock that let one profiled run at a time (`one_run`) is gone. A corpus run holds its run's records
    (`hold_records`); one nested in it neither resets nor writes them.
  - The threads' CPU accounts (`THREADS`, `EXITED`, `EXITED_GAPS`) were to move with the pool (step 5). They stay
    with the threads, the process's: a machine's pool threads are threads of the process (step 5, below).
  - `WAVES` (an id counter) and `WARM_FAULTS` (the process's exit status under `RERAIL_WARM=list`, which an atexit
    handler reads) stay (step 5's list, below); which wave a thread's writes join is the run's context
    (`write_back::RunWaves`).
- Step 4 (the connection), first part:
  - Each connection holds the engine's values for it in its SQLite client data (`vtab::ConnData`, by type). They are
    dropped as it closes, after `Forget::scope` has taken what it frees on a thread of its own.
  - A `Db` points at them. An attached `Db` borrows its connection, which cannot close meanwhile. A detached one (a
    stratum's job off the connection's thread) is made with the `Arc` its holder keeps (`plan::Strand`'s last field).
  - setfix's per-connection maps (COMPONENTS, DERIVED, ADOPTED, INPUTS, CHANGING, SCOPE_PATHS, SCOPE_SYMS), each a
    process map keyed by the connection's address, are one `OnConn` on the connection. `accel::sharded::ByScope`,
    their sharded lock, is gone with them.
- Step 4, second part:
  - vtab's BOUND, REGISTERED, VIEWS, UNRESOLVED and SETUP are one `OnVtab` on the connection. A `Modules` keeps weak
    handles to the connections it bound on, for its drop. A node read's plan step holds its connection's data
    weakly (`NodeTexts::At`), and the SQL `node_read` functions hold it (they close with the connection).
  - plan's TEMP_LEFT (what a connection's runs left in SQL) is on the connection (`plan::Left`).
  - atoms' RESERVED was keyed by the arena's address. It is the arena's own (`Atoms::reserved`), held by each
    reservation of its atoms.
  - setfix's STORED is keyed by database, and every connection on one store shares its tables. So it is the store's,
    shared by design: it moves with the store's resources (step 5).
- Step 5 (`Shared`), the mechanism:
  - `profile::Shared` is a typed store of what is shared by design, made by a caller. A run carries the one it is
    handed (`run::scope_in(Some(&shared), …)`), and code reads it through the run (`run::shared::<T>()`), from any
    crate. A run handed none has one of its own, as apart from every other run as one in another process (the store
    is already safe across processes: its lease).
  - The CLI's command makes one (`tools::cli`), the reactor one per instance, its one caller. A caller of
    `tools::cli_in` passes its own. Node's entry points had none at first; they have one now (the Node host, below).
  - Outside a run, `shared::<T>()` gives a value nothing keeps. That is wrong for an interner or a per-file registry,
    whose ids must agree wherever they are read, so those move only once every reader is inside a run or holds the
    object (each group's move says which).
- Step 5, the machine (the user, 2026-11-19): the machine's resources are a second explicit object, `Machine`: the CPU
  job pool and its queues, the pool threads' CPU accounts, the measured roofline, and the cost and price model learned
  from this machine's runs. A caller makes it and every `Shared` is made on one (`Shared::on(&machine)`), so callers of
  different trust can share one pool and one model of the machine while holding different `Shared` objects. A job on
  the pool carries its own caller's run and `Shared`, so nothing crosses between callers through the pool. A `Shared`
  made with none has a machine of its own.
- Step 6 (classification by hand): the check now covers the accel crate too (`accel/src`: the pool, the device, the
  price and cost tables), and has a `machine` class. Every entry was read and classed: of 482, 249 stay (constants
  computed once from the engine's own files, code registered at load, id counters, test locks), 51 are run data
  (mostly the profile's counters and settings), 89 are shared by design (interners, caches of callers' programs, the
  store's registries) and 93 are the machine's (the pool, the device's pipelines and memory ledger, the learned cost
  and price model, the process's sampler). The three moving classes are moved group by group (step 5).
- Step 5, as done (2026-11-19). `global-state.txt` holds only `stays` (293 entries); no class that moves is left.
  At its merge into the sync branch (2026-11-19) the statics that branch had added were classed, each with its
  reason, and two left as `shared`: the pinned Git revisions (`git_snapshot`'s `VIEWS` and its count), a read under
  a root from any run in the process goes to them (work queue Q352).
  - What moved:
    - The machine's: the pool's accounts, the device and its pipelines, the price and cost tables, the lanes, the
      batch pricer, the roofline, the CPU slot for workspace work.
    - The shared object's: every interner (call names, contracts, the facts' symbols, the front's relation names,
      relation numbers, the typing rounds' large constants); every store registry (atoms by file and the in-memory
      atoms, file identities, canonical paths, connections and handles, frames' counts and snapshots, the journal's
      streams, observers, writers and their account, the content stores, setfix's symbols and stored tables); the
      caches of callers' programs; the join statistics and their search job.
    - The run's: the git calls' state, the lanes' counts, the profile's counters.
  - Two objects on one store behave as two processes do: each has its own writer, atoms, snapshots and connections
    (a thread's connections are kept by object and path), and the store's lease keeps them apart.
  - A thread the engine starts reads its starter's object: `exec::service`, the compile queue's jobs, the warm-up,
    the join search's upkeep job (registered once per object and holding it), a workspace's tasks and its sync
    methods (`within`).
  - Hot readers borrow a shared value (`with_shared`, as `with_local`): no lock and no count of references, so the
    sharded relation numbers (decision 0279 §7) stay free of a shared word.
  - The front's `Name` is equal by address, or by its text where the names come from two tables or a static, so a
    name a static holds still equals the same name from an object.
  - The Node host: the addon's caller is the Node environment. It makes one `Machine` and keeps it in its instance
    data, and each workspace it makes is on a `Shared` of its own on that machine, so an environment's workspaces
    share one pool, one CPU slot and one learned model. A worker thread's environment has its own. A Rust caller with
    no environment (`CodeRuleDslWorkspace::new`) takes `default_shared`.
  - The strict run (`RERAIL_STRICT_RUN=1`, now every product run: the refusal below) logs each read outside any run
    once, by type, with its backtrace, to the engine's stderr. The B-now policy run (corpus0) logs none, with its 463 findings unchanged (2026-11-19: a Linux
    profiling build, and the Metal path on an M3 Max). On Metal the last were a pooled device buffer's return,
    dropped outside any run: a lease keeps its machine's device memory and budgets, and its return trims the pool to
    those (`gpu::shared::Pooled`). The other last reads were the ones on threads that took a job
    without its run: a stream's queued job (`accel::stream`, from its start to its finish on the scoreboard), the
    cores' launches (`exec`'s pool threads), the keeps' per-relation pricing (`plan::keeps`, a `par_iter`), and the
    services' list of a service started outside any run (the signal watcher, which is now listed in no machine).
- Step 5, the departures:
  - A read outside any run was not refused at first (2026-11-19, before the strict runs below). It is now
    (2026-11-19, after them), as Decision 2 says, by build:
    - **A product build:** the read takes a value of its own, which reaches nothing of any caller's and which nothing
      keeps, and is said once a type on stderr with its backtrace. The command fails at its end (`tools::cli`,
      exit 3), its output written, as a run cut at the width cap does. These readers return values, not results, so
      the refusal is the command's.
    - **A test build (`test-hooks`):** the read panics on a thread the engine made (its pools' workers, the cost
      model's pool, `exec`'s services: each marks itself, `rerail_context::engine_thread`), where a miss is a job or
      service that lost its caller. On any other thread the reader is a test calling the engine's parts directly, a
      caller with no entry point, and its object is the tests' one (`TESTS`; a thread value is a fresh default).
      The first quick tier with a blanket panic failed 2,361 tests, nearly all a test's own reads; the rest were on
      threads the tests started (each now holds its starter's object or run: `profile::caller`) and on the engine's
      pools, where a job of a caller with no run carried none, or a parallel loop carried nothing (the demand
      walks and workspace sizes, the GPU's prewarm: each now carries its caller).
    - **A run that has ended leaves no values behind** on a thread that ran a part of it: a thread's copy of a run's
      values (`SEEN`) is dropped when the run leaves the thread and nothing holds it (`left_run`, `run_ended`), not
      when the thread next enters a run or exits. Kept, a finished run's values (its device leases, its blocks) were
      pinned on an idle pool thread until it ran another run's job.
  - **A job pool per machine** (2026-11-19). A machine's pool is its own (`rerail_context::Cores`, a machine value
    made at its first job, `threads` threads named `rerail-cpu <i>`); rayon's global pool is never built. Rayon's
    functions reach the pool of the thread they run on, else the global one, so every parallel loop and every rayon
    call that starts work goes through the machine's entry points: `on_cores(|| …)` (on a pool's thread it runs
    there; else on the caller's machine's pool, in the caller's run and context, while the caller waits),
    `cores_spawn`, `cores_spawn_fifo`, `cores_scope`, `cores_threads`. 110 loops in 51 files were put inside them,
    forest's included (its prediction ran on the caller's thread). `scripts/check/check_machine_pool.py`
    (pre-commit) refuses a loop or call outside them, `build_global`, and a pool made where its list says no reason.
  - **Every job carries its maker's run** (2026-11-20, Q300). A loop splits by rayon's `join`, and a half another
    pool thread steals ran there with no run and no bindings. rayon-core is patched (`native/vendor/rayon-core-1.13.0`,
    PATCHES.md) so a job captures what its maker's thread carries when made (a `join` half, an `install`ed closure, a
    spawned or scoped job) and enters it on the thread that runs it, left before the job says it is done. The engine's
    carrier (`rerail_context::carry`) is `job_run`, installed when a machine is made; a thread with no run carries
    nothing. It replaced `Inside`, the runs listed in a pool, which a stolen half could read only while every job in
    the pool was one run's: a corpus run's phase 0 beside its main program, profiled, was refused there. corpus0 x
    B-now, cold: 1,143.9 G instructions with it, 1,146-1,155 G before (an M3 under load); profiled, it completes.
  - A program's main thread is one of its machine's pool's (`Machine::here`): the CLI's `main` makes its machine so
    and hands it to the command (`tools::cli`), and Node's main environment makes its machine so on its first
    workspace (`environment_machine`; any other environment's pool has threads of its own). Rayon keeps a pool's
    hold on such a thread, so only the thread that lives as long as the process makes one.
  - The threads' CPU accounts (`THREADS`, `EXITED`, `EXITED_GAPS`) stay with the threads, the process's: a
    machine's pool threads are threads of the process, and the accounts measure the process as the OS reports it.
  - Stays, each with its reason in `global-state.txt`:
    - the heap sampler and the footprint sampler (they measure the process, as the OS reports it);
    - `RESERVED` (the process's own mappings, its address space);
    - `POLICY` (the block allocator's policy, registered at load);
    - process-unique name and object counters;
    - test-only counters;
    - `WARM_FAULTS` (the exit status an atexit handler reads);
    - `WAVES` (an id counter);
    - the leaked names of the engine's own counters and fallback reasons;
    - the typing settings' interner. It holds immutable values and a `Reading` compares them by value, so no id of
      it can differ between callers.
