// How the shippability bar (scripts/ship/shipbar.ts) holds one process to --cpus, caps its memory and time, and
// measures it (wall, CPU user + sys, peak memory) on each platform, and which method did each of those: every run row
// records it, so the report says what was pinned and capped where.
//
//   linux   taskset pins; GNU time measures (peak RSS of the tree); a systemd user scope caps memory (cgroup
//           MemoryMax, no swap); coreutils `timeout` caps time. Per-CPU busy time from /proc/stat, load from
//           /proc/loadavg.
//   darwin  macOS has no CPU affinity: runs are not pinned, and RERAIL_THREADS holds the engine to as many threads as
//           --cpus names, on whichever cores the scheduler picks. /usr/bin/time -l measures (peak RSS);
//           platform-watch.mjs caps time, and memory by polling the tree's RSS (no kernel limit: a run can overshoot
//           the cap by what it allocates between two polls). Load from getloadavg; no per-CPU busy time.
//   win32   a job object (platform-win.cs, compiled with the .NET Framework's csc on first use) pins (job affinity),
//           caps memory (the job's committed memory) and time (the job is terminated), and measures CPU (the job's
//           accounting) and peak memory (the root process's peak working set). Per-CPU busy time from
//           NtQuerySystemInformation; Windows keeps no load average.
//
// The engine runs RERAIL_THREADS = the number of --cpus on every platform, which on Linux is what taskset already
// gives it: a candidate sizing its pool from the machine's cores then runs as many threads as the baseline's fixed pool.

import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, rmSync } from "node:fs";
import { cpus as logicalCpus, hostname, loadavg, release, version } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));

/** How one run was held to its CPUs, capped and measured. */
export interface RunMethod {
  platform: "linux" | "darwin" | "win32";
  /** taskset: hard affinity; job-affinity: a Windows job object's affinity; none: not pinned (macOS). */
  pinning: "taskset" | "job-affinity" | "none";
  measured: "gnu-time" | "bsd-time" | "job-accounting";
  /** max-rss: the tree's peak resident set (getrusage); peak-working-set: the root process's peak working set. */
  peak: "max-rss" | "peak-working-set";
  /** cgroup and job-commit are kernel limits; polled-rss kills the tree when its sampled RSS passes the cap. */
  memoryCap: "cgroup" | "job-commit" | "polled-rss";
  timeCap: "timeout" | "job" | "watchdog";
}

/** Linux's method: what every row written before rows recorded their method was measured with. */
export const LINUX_METHOD: RunMethod = { platform: "linux", pinning: "taskset", measured: "gnu-time", peak: "max-rss", memoryCap: "cgroup", timeCap: "timeout" };
const DARWIN_METHOD: RunMethod = { platform: "darwin", pinning: "none", measured: "bsd-time", peak: "max-rss", memoryCap: "polled-rss", timeCap: "watchdog" };
const WIN32_METHOD: RunMethod = { platform: "win32", pinning: "job-affinity", measured: "job-accounting", peak: "peak-working-set", memoryCap: "job-commit", timeCap: "job" };

/** The method's gaps against Linux's pinned, kernel-capped runs, in words; empty when there are none. */
export function methodGaps(method: RunMethod): string[] {
  const gaps: string[] = [];
  if (method.pinning === "none") gaps.push("not pinned: macOS has no CPU affinity, so RERAIL_THREADS holds the engine to the --cpus count of threads on any core, and other load shares its cores");
  if (method.memoryCap === "polled-rss") gaps.push("memory cap polled, not enforced by the kernel: the tree's RSS is sampled every 0.5 s and the tree killed above the cap");
  if (method.peak === "peak-working-set") gaps.push("peak memory is the root process's peak working set (Windows keeps no peak RSS of a process tree)");
  if (method.platform === "win32") gaps.push("no load average on Windows: the busiest pinned and idle CPUs over each run are recorded instead");
  return gaps;
}

/** Each distinct method among the rows, with how many rows it measured, most first. */
export function methodCounts(rows: readonly { method: RunMethod }[]): { method: RunMethod; runs: number }[] {
  const counts = new Map<string, { method: RunMethod; runs: number }>();
  for (const { method } of rows) {
    const key = JSON.stringify([method.platform, method.pinning, method.measured, method.peak, method.memoryCap, method.timeCap]);
    const entry = counts.get(key) ?? { method, runs: 0 };
    entry.runs += 1;
    counts.set(key, entry);
  }
  return [...counts.values()].sort((a, b) => b.runs - a.runs);
}

/** One measured process. */
export interface MeasureRequest {
  command: readonly string[];
  cwd: string;
  env: Record<string, string>;
  cpus: readonly number[];
  siblings: readonly number[];
  /** systemd's size syntax: 24G, 512M. */
  memCap: string;
  timeCapS: number;
  /** Open file descriptors the process writes its stdout and stderr to. */
  stdout: number;
  stderr: number;
  /** A path prefix for the measurement's own scratch files, removed before return. */
  scratch: string;
}

export interface Measured {
  /** 137 when a cap killed the run. */
  exit: number;
  /** The cap that killed the run, when the platform knows; null otherwise (Linux tells only by the wall time). */
  killedBy: "time" | "memory" | null;
  wallS: number;
  userS: number;
  sysS: number;
  peakKB: number;
  /** The one-minute load average as the run started; null where the platform keeps none (Windows). */
  load: number | null;
  /** The busiest pinned CPU and the busiest idle sibling over the run (fraction busy); null where runs are not pinned. */
  pinnedBusy: number | null;
  siblingBusy: number | null;
}

/** A host the runs were made on, as `run` found it (<out>/machines/<host>.json). */
export interface MachineInfo {
  host: string;
  cpu_model: string | null;
  logical_cpus: number;
  kernel: string;
  os: string | null;
  /** The EC2 instance type, when the instance metadata service answers. */
  instance_type: string | null;
}

export interface Platform {
  method: RunMethod;
  /** The pinned CPUs' hyperthread siblings, which must stay idle; refuses a set holding two threads of one core. */
  siblings(cpus: readonly number[]): number[];
  measure(request: MeasureRequest): Measured;
  machine(): MachineInfo;
}

// ---------------------------------------------------------------- pure parts

/** A CPU list as taskset and sysfs write it: "8-11", "0,2,4-6". */
export function cpuList(spec: string): number[] {
  const cpus = new Set<number>();
  for (const part of spec.split(",")) {
    const [from, to, ...rest] = part.split("-").map((n) => (n.trim() === "" ? Number.NaN : Number(n)));
    if (from === undefined || !Number.isInteger(from) || from < 0 || rest.length > 0 || (to !== undefined && (!Number.isInteger(to) || to < from))) throw new Error(`${spec} is not a CPU list`);
    for (let cpu = from; cpu <= (to ?? from); cpu += 1) cpus.add(cpu);
  }
  return [...cpus].sort((a, b) => a - b);
}

/** Bytes in a size as systemd's MemoryMax takes it: a number with an optional K, M, G or T (powers of 1024). */
export function capBytes(size: string): number {
  const match = /^(\d+(?:\.\d+)?)([KMGT]?)$/i.exec(size.trim());
  if (match === null) throw new Error(`--mem-cap ${size} is not a size (24G, 512M)`);
  const power = " KMGT".indexOf((match[2] || " ").toUpperCase());
  return Math.floor(Number(match[1]) * 1024 ** power);
}

/** The idle siblings of the pinned CPUs, given each physical core's logical CPUs. */
export function idleSiblingsOf(cpus: readonly number[], cores: readonly (readonly number[])[]): number[] {
  const siblings = new Set<number>();
  for (const cpu of cpus) {
    const core = cores.find((threads) => threads.includes(cpu));
    if (core === undefined) throw new Error(`no CPU ${cpu} on this machine`);
    for (const sibling of core) {
      if (sibling === cpu) continue;
      if (cpus.includes(sibling)) throw new Error(`--cpus holds CPUs ${cpu} and ${sibling}, two threads of one core; pin one thread per physical core`);
      siblings.add(sibling);
    }
  }
  return [...siblings].sort((a, b) => a - b);
}

export type CpuTimes = Map<number, { busy: number; total: number }>;

/** The busiest of `cpus` between two samples, as a fraction busy to three places; 0 for none. */
export function busiest(before: CpuTimes, after: CpuTimes, cpus: readonly number[]): number {
  let most = 0;
  for (const cpu of cpus) {
    const a = before.get(cpu);
    const b = after.get(cpu);
    if (a === undefined || b === undefined || b.total === a.total) continue;
    most = Math.max(most, (b.busy - a.busy) / (b.total - a.total));
  }
  return Math.round(most * 1000) / 1000;
}

/** /proc/stat's per-CPU lines as busy and total jiffies. */
export function parseProcStat(text: string): CpuTimes {
  const times: CpuTimes = new Map();
  for (const line of text.split("\n")) {
    const match = /^cpu(\d+)\s+(.*)$/.exec(line);
    if (match === null) continue;
    const fields = match[2]!.split(/\s+/).map(Number);
    const total = fields.slice(0, 8).reduce((a, b) => a + b, 0);
    times.set(Number(match[1]), { busy: total - (fields[3] ?? 0) - (fields[4] ?? 0), total });
  }
  return times;
}

/** GNU time's `-f "%e %U %S %M"` line: seconds and KB; zeros when the run left none. */
export function parseGnuTime(text: string): { wallS: number; userS: number; sysS: number; peakKB: number } {
  const lines = text.split("\n").map((line) => line.trim()).filter((line) => line !== "");
  const [wallS = 0, userS = 0, sysS = 0, peakKB = 0] = (lines.at(-1) ?? "").split(/\s+/).map(Number);
  return { wallS, userS, sysS, peakKB };
}

/** BSD time's `-l` report (macOS): the real/user/sys line and the peak RSS, which macOS gives in bytes. */
export function parseBsdTime(text: string): { wallS: number; userS: number; sysS: number; peakKB: number } | null {
  const times = /([\d.]+)\s+real\s+([\d.]+)\s+user\s+([\d.]+)\s+sys/.exec(text);
  const rss = /(\d+)\s+maximum resident set size/.exec(text);
  if (times === null || rss === null) return null;
  return { wallS: Number(times[1]), userS: Number(times[2]), sysS: Number(times[3]), peakKB: Math.round(Number(rss[1]) / 1024) };
}

/** What platform-win.cs writes after a run. */
export interface LauncherResult {
  exit: number;
  killedBy: "time" | "memory" | null;
  wallMs: number;
  userMs: number;
  sysMs: number;
  /** The root process's peak working set; 0 when Windows would not say. */
  peakWorkingSetKB: number;
  /** The largest committed memory of any process in the job. */
  peakCommitKB: number;
  pinnedBusy: number;
  siblingBusy: number;
}

export function fromLauncher(result: LauncherResult): Omit<Measured, "load"> {
  return {
    exit: result.killedBy === null ? result.exit : 137,
    killedBy: result.killedBy,
    wallS: result.wallMs / 1000,
    userS: result.userMs / 1000,
    sysS: result.sysMs / 1000,
    // A process whose working set Windows will not report after exit falls back to its committed memory.
    peakKB: result.peakWorkingSetKB > 0 ? result.peakWorkingSetKB : result.peakCommitKB,
    pinnedBusy: result.pinnedBusy,
    siblingBusy: result.siblingBusy,
  };
}

// ---------------------------------------------------------------- the platforms

function run(command: string, args: readonly string[]): { status: number; stdout: string; stderr: string } {
  const result = spawnSync(command, args, { encoding: "utf8", maxBuffer: 1 << 26 });
  return { status: result.status ?? (result.error === undefined ? 1 : 127), stdout: result.stdout ?? "", stderr: result.stderr ?? String(result.error ?? "") };
}

function readText(path: string): string {
  return existsSync(path) ? readFileSync(path, "utf8") : "";
}

/** The EC2 instance type, when the instance metadata service answers (curl ships with Linux, macOS and Windows). */
function instanceType(): string | null {
  const token = run("curl", ["-s", "-m", "1", "-X", "PUT", "http://169.254.169.254/latest/api/token", "-H", "X-aws-ec2-metadata-token-ttl-seconds: 60"]);
  const type = token.status === 0 && token.stdout !== "" ? run("curl", ["-s", "-m", "1", "-H", `X-aws-ec2-metadata-token: ${token.stdout}`, "http://169.254.169.254/latest/meta-data/instance-type"]).stdout.trim() : "";
  return /^[a-z0-9-]+\.[a-z0-9-]+$/.test(type) ? type : null;
}

function linux(): Platform {
  if (run("taskset", ["-c", "0", "true"]).status !== 0) throw new Error("taskset (util-linux) is missing");
  const time = run("/usr/bin/time", ["--version"]);
  if (!/GNU/.test(time.stdout + time.stderr)) throw new Error("/usr/bin/time is not GNU time");
  if (run("systemd-run", ["--user", "--scope", "--quiet", "-p", "MemoryMax=1G", "true"]).status !== 0) throw new Error("systemd-run --user --scope does not work here (no user manager?); it caps each run's memory");
  const procStat = (): CpuTimes => parseProcStat(readFileSync("/proc/stat", "utf8"));
  return {
    method: LINUX_METHOD,
    siblings(cpus) {
      const cores = cpus.map((cpu) => {
        const path = `/sys/devices/system/cpu/cpu${cpu}/topology/thread_siblings_list`;
        if (!existsSync(path)) throw new Error(`no CPU ${cpu} (${path})`);
        return cpuList(readFileSync(path, "utf8").trim());
      });
      return idleSiblingsOf(cpus, cores);
    },
    measure(r) {
      const timeFile = `${r.scratch}.time`;
      const args = [
        "-c", r.cpus.join(","),
        "/usr/bin/time", "-f", "%e %U %S %M", "-o", timeFile,
        // GNU time stays outside the cgroup, so a run the cap kills is still measured (time to the kill, peak RSS).
        "systemd-run", "--user", "--scope", "--quiet", "-p", `MemoryMax=${r.memCap}`, "-p", "MemorySwapMax=0",
        "timeout", "-s", "KILL", String(r.timeCapS),
        ...r.command,
      ];
      const load = Number(readFileSync("/proc/loadavg", "utf8").split(" ")[0]);
      const before = procStat();
      const child = spawnSync("taskset", args, { cwd: r.cwd, env: r.env, stdio: ["ignore", r.stdout, r.stderr] });
      const after = procStat();
      const figures = parseGnuTime(readText(timeFile) || "0 0 0 0");
      rmSync(timeFile, { force: true });
      return { exit: child.status ?? 137, killedBy: null, ...figures, load, pinnedBusy: busiest(before, after, r.cpus), siblingBusy: busiest(before, after, r.siblings) };
    },
    machine() {
      const cpuModel = existsSync("/proc/cpuinfo") ? (/^model name\s*:\s*(.+)$/m.exec(readFileSync("/proc/cpuinfo", "utf8"))?.[1] ?? null) : null;
      const os = existsSync("/etc/os-release") ? (/^PRETTY_NAME="?([^"\n]+)"?$/m.exec(readFileSync("/etc/os-release", "utf8"))?.[1] ?? null) : null;
      return { host: hostname(), cpu_model: cpuModel, logical_cpus: logicalCpus().length, kernel: release(), os, instance_type: instanceType() };
    },
  };
}

function darwin(): Platform {
  if (run("/usr/bin/time", ["-l", "-o", "/dev/null", "true"]).status !== 0) throw new Error("/usr/bin/time -l -o does not work here");
  return {
    method: DARWIN_METHOD,
    siblings(cpus) {
      if (cpus.length > logicalCpus().length) throw new Error(`--cpus names ${cpus.length} CPUs; this Mac has ${logicalCpus().length}`);
      return [];
    },
    measure(r) {
      const timeFile = `${r.scratch}.time`;
      const resultFile = `${r.scratch}.watch.json`;
      const load = Math.round(loadavg()[0]! * 100) / 100;
      spawnSync(process.execPath, [join(HERE, "platform-watch.mjs"), resultFile, String(r.timeCapS), String(capBytes(r.memCap)), "--", "/usr/bin/time", "-l", "-o", timeFile, ...r.command], {
        cwd: r.cwd,
        env: r.env,
        stdio: ["ignore", r.stdout, r.stderr],
      });
      const watch = JSON.parse(readText(resultFile) || '{"exit":137,"killedBy":null,"wallS":0,"peakKB":0}') as { exit: number; killedBy: "time" | "memory" | null; wallS: number; peakKB: number };
      // time writes its report before it exits, also for a child the watchdog killed; the watchdog's own figures
      // stand in where it wrote none.
      const figures = parseBsdTime(readText(timeFile)) ?? { wallS: watch.wallS, userS: 0, sysS: 0, peakKB: watch.peakKB };
      rmSync(timeFile, { force: true });
      rmSync(resultFile, { force: true });
      return { exit: watch.killedBy === null ? watch.exit : 137, killedBy: watch.killedBy, ...figures, peakKB: Math.max(figures.peakKB, watch.killedBy === "memory" ? watch.peakKB : 0), load, pinnedBusy: null, siblingBusy: null };
    },
    machine() {
      const productVersion = run("sw_vers", ["-productVersion"]).stdout.trim();
      return {
        host: hostname(),
        cpu_model: run("sysctl", ["-n", "machdep.cpu.brand_string"]).stdout.trim() || null,
        logical_cpus: logicalCpus().length,
        kernel: release(),
        os: productVersion === "" ? null : `macOS ${productVersion}`,
        instance_type: instanceType(),
      };
    },
  };
}

/** The launcher, compiled from platform-win.cs into <scratch dir> once per source version. */
function windowsLauncher(scratchDir: string): string {
  const source = join(HERE, "platform-win.cs");
  const digest = createHash("sha256").update(readFileSync(source)).digest("hex").slice(0, 12);
  const exe = join(scratchDir, `shipbar-launch-${digest}.exe`);
  if (existsSync(exe)) return exe;
  const windir = process.env.WINDIR ?? process.env.SystemRoot ?? "C:\\Windows";
  const csc = join(windir, "Microsoft.NET", "Framework64", "v4.0.30319", "csc.exe");
  if (!existsSync(csc)) throw new Error(`${csc} is missing: the .NET Framework 4 compiler builds the job-object launcher`);
  mkdirSync(scratchDir, { recursive: true });
  const built = run(csc, ["/nologo", "/optimize+", "/platform:x64", `/out:${exe}`, source]);
  if (built.status !== 0 || !existsSync(exe)) throw new Error(`compiling ${source} failed: ${built.stdout}${built.stderr}`);
  return exe;
}

function win32(scratchDir: string): Platform {
  const launcher = windowsLauncher(scratchDir);
  return {
    method: WIN32_METHOD,
    siblings(cpus) {
      const topology = run(launcher, ["topology"]);
      if (topology.status !== 0) throw new Error(`${launcher} topology: ${topology.stderr}`);
      const cores = JSON.parse(topology.stdout) as number[][];
      if (cpus.some((cpu) => cpu > 63)) throw new Error("--cpus above 63: the launcher pins within processor group 0");
      return idleSiblingsOf(cpus, cores);
    },
    measure(r) {
      const resultFile = `${r.scratch}.launch.json`;
      spawnSync(launcher, ["run", resultFile, r.cpus.join(","), r.siblings.join(","), String(capBytes(r.memCap)), String(r.timeCapS), "--", ...r.command], {
        cwd: r.cwd,
        env: r.env,
        stdio: ["ignore", r.stdout, r.stderr],
        windowsHide: true,
      });
      const text = readText(resultFile);
      rmSync(resultFile, { force: true });
      if (text === "") return { exit: 127, killedBy: null, wallS: 0, userS: 0, sysS: 0, peakKB: 0, load: null, pinnedBusy: null, siblingBusy: null };
      return { ...fromLauncher(JSON.parse(text) as LauncherResult), load: null };
    },
    machine() {
      return { host: hostname(), cpu_model: logicalCpus()[0]?.model.trim() ?? null, logical_cpus: logicalCpus().length, kernel: release(), os: version(), instance_type: instanceType() };
    },
  };
}

/** This machine's platform, or why it cannot measure a run; `scratchDir` holds what the platform builds once. */
export function openPlatform(scratchDir: string): Platform {
  if (process.platform === "linux") return linux();
  if (process.platform === "darwin") return darwin();
  if (process.platform === "win32") return win32(scratchDir);
  throw new Error(`run measures on Linux, macOS and Windows; this is ${process.platform}`);
}
