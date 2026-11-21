# An elastic farm: the plan

How the farm takes more parallel work without overloading a machine (2026-11-20). This is a plan:
`README.md` says what the farm does today. Steps 1 to 3 are in the order to build them.

## Where the farm is today

- **Capacity is fixed.** The farm has its machines and their guests: VMs it starts when a command
  would wait, and stops when idle. A cloud VM (the CUDA work's V100) runs only commands that name
  it. Nothing grows the farm when every slot is taken.
- **A host runs a third of its cores in commands** (`test_slots`), and nothing bounds what each
  command uses. Each command's cargo builds on every core, and nextest runs on every core too. The
  dashboard on 2026-11-20 showed m3 (16 cores) at load 51 and 100% CPU with three test commands.
  Those commands held 2.5 to 6.4 GB each, and one used 14 cores.
- **No memory check, no memory cap.** A slot is taken whatever memory the host has left. A command
  that grows is never stopped.
- **The GPU is unguarded.** This Mac went down on 2026-11-20. WindowServer stopped answering for
  40 s, and macOS's watchdog restarted the machine. Memory was not the cause: processes held
  74 GB of 128. At the time, 16 Metal shader compilers were running, and earlier samples had the
  GPU at 98% busy with Metal tests. A saturated GPU starving the display is the likely cause,
  though the report cannot prove it.
- **Every host builds its own copy.** A command compiles the tests in that host's own target
  directory. A cold host, such as a new cloud VM, spends a whole test build before its first test.
  Each crate variant also costs about 2 GB of its disk.

## What Tim's work brings

- **CI #830, build once and run on many.** CI compiles the engine's test binaries once
  (`rust-tests-build`), packs them as a 0.8 GB artifact, and runs them on several legs. Every
  build starts warm from main's targets and compile caches, saved as artifacts. A change runs only
  the crates and tests it can affect (`rust-selection.mjs`, `test-selection.mjs`).
- **The shippability bar's fan-out (`scripts/ship/fanout.ts`) over AWS hosts** (c7i.8xlarge,
  reached by ssh names, made outside this repository):
  - The work is split into units of at most about 4 minutes, placed longest first on the least
    loaded host (LPT).
  - Each host runs one worker per pinned 4-core set. A worker claims the next unit with an atomic
    `mkdir`, so any free worker takes the next unit.
  - The engines are built once, on the first host, and copied to the others.
  - Jobs run detached on the hosts (`setsid nohup`), the Mac polls them, and the run resumes where
    it stopped.

The two ideas worth taking: **build once and ship the binaries**, and **units that free workers
claim**. With both, a host that joins mid-run is useful at once.

## Step 1: budgets, so no machine is overloaded

- **CPU share.** A slot's command gets `cores / slots × 2` jobs (`CARGO_BUILD_JOBS`,
  `NEXTEST_TEST_THREADS`). A host is then loaded to at most twice its cores, not three or four
  times. A command that runs alone on its host takes all the cores.
- **Memory admission.** A slot is taken only while the host has the command's expected peak
  available, plus a reserve. The agent records each command's peak memory, by the command without
  its paths; the expected peak is the last few peaks, or 4 GB for a new command. A host short of
  memory counts as busy, so the command goes elsewhere or waits.
- **A memory cap.** The agent watches its command's process tree. A tree over its budget (the
  host's memory less the reserve, shared among the running commands) is stopped, and its exit
  says why. On Linux the cap is a cgroup (`systemd-run --user --scope -p MemoryMax=`) where one is
  available.
- **One GPU command per machine.** A command that uses the GPU (Metal or CUDA tests, declared with
  `--gpu`, or recognised by its features) takes the machine's one GPU slot. This Mac runs no GPU
  test command while one already runs here.
- **This Mac keeps its headroom.** A command runs here, when no host can take it, only with the
  same memory and GPU checks. Otherwise it waits for the farm.

## Step 2: build once, run on many

`test-remote --spread -- cargo nextest run --lib [--profile gate]` would work like this:

1. **One build per system and architecture.** One host of each (os, arch) pair builds a nextest
   archive in its warm target directory (`cargo nextest archive`). The pairs are macOS arm64,
   Linux aarch64 and Linux x86_64; Windows comes later. An archive is about 1 GB compressed.
2. **Partitions as units.** The run is cut into about four partitions per free slot
   (`--partition count:K/M`). A free slot of the same (os, arch) claims the next partition, with
   its archive and the checkout's files, and runs it (`run --archive-file`, `--workspace-remap`).
   A host whose slot frees, or a guest that comes up mid-run, claims what is left.
3. **One report.** Exit statuses and JUnit results are merged into one, and a failing test is
   named with the host that ran it.

A cold host, a new cloud VM included, then needs the archive and the checkout: seconds to a
minute, not a cold compile and its disk.

## Step 3: an elastic pool of cloud hosts

- **An `ec2` guest kind,** beside `gce`: start, stop and state through `aws ec2`, on instances
  that are stopped when idle. Their disks keep the checkout copy and the warm targets, so a
  stopped host costs only its disk.
- **Burst.** `"burst": true` cloud guests are started when a command would wait more than a set
  time with every slot taken, as local guests are today. A spread run asks for as many as its
  partitions can use.
- **Limits.** The cloud guests running at once, instance-hours a day, and the idle stop
  (10 minutes, as today). Spot instances for burst, if the interruptions are acceptable: a
  partition whose host goes away is claimed again.
- **What the farm needs.** Access to Tim's hosts, or to an AWS account that can make them, and
  how they are reached (ssh names, SSM). No identifiers go in this repository: like every host,
  they live in the farm's local configuration.
