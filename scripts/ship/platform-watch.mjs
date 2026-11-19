// macOS's time and memory caps for the shippability bar (scripts/ship/platform.ts): runs one command, kills its
// process tree past the time cap, or once the tree's resident memory, sampled every 0.5 s, passes the memory cap, and
// writes what it saw. macOS has no cgroups and enforces no RLIMIT_AS, so the memory cap is this sample: a run can
// overshoot it by what it allocates between two samples.
//
//   node scripts/ship/platform-watch.mjs <result.json> <time cap s> <memory cap bytes> -- <command> [args...]
//
// The command is /usr/bin/time -l wrapping the measured process: a cap kills the processes below time, so time still
// reports the run up to the kill. Writes { exit, killedBy: "time" | "memory" | null, wallS, peakKB }, peakKB being the
// highest sampled RSS of the tree.

import { execFileSync, spawn } from "node:child_process";
import { writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

const POLL_MS = 500;

/**
 * The processes below `root` in `ps -A -o pid=,ppid=,rss=` output (RSS in KB), each with its RSS; `root` itself is
 * left out (it is the measuring time, not the measured run).
 */
export function descendants(ps, root) {
  const children = new Map();
  const rss = new Map();
  for (const line of ps.split("\n")) {
    const [pid, ppid, kb] = line.trim().split(/\s+/).map(Number);
    if (!Number.isInteger(pid) || !Number.isInteger(ppid)) continue;
    rss.set(pid, Number.isFinite(kb) ? kb : 0);
    if (!children.has(ppid)) children.set(ppid, []);
    children.get(ppid).push(pid);
  }
  const found = [];
  const queue = [...(children.get(root) ?? [])];
  while (queue.length > 0) {
    const pid = queue.shift();
    found.push({ pid, rssKB: rss.get(pid) ?? 0 });
    queue.push(...(children.get(pid) ?? []));
  }
  return found;
}

function main() {
  const [result, timeCap, memoryCap, separator, ...command] = process.argv.slice(2);
  if (result === undefined || separator !== "--" || command.length === 0) {
    console.error("usage: platform-watch.mjs <result.json> <time cap s> <memory cap bytes> -- <command> [args...]");
    process.exit(2);
  }
  const capKB = Number(memoryCap) / 1024;
  const started = process.hrtime.bigint();
  const child = spawn(command[0], command.slice(1), { stdio: "inherit" });
  let killedBy = null;
  let peakKB = 0;
  const tree = () => {
    try {
      return descendants(execFileSync("ps", ["-A", "-o", "pid=,ppid=,rss="], { encoding: "utf8" }), child.pid);
    } catch {
      return [];
    }
  };
  const kill = (cap) => {
    killedBy ??= cap;
    for (const { pid } of tree()) {
      try {
        process.kill(pid, "SIGKILL");
      } catch {
        // Gone already.
      }
    }
  };
  const timer = setTimeout(() => kill("time"), Number(timeCap) * 1000);
  const poll = setInterval(() => {
    const kb = tree().reduce((sum, p) => sum + p.rssKB, 0);
    peakKB = Math.max(peakKB, kb);
    if (kb > capKB) kill("memory");
  }, POLL_MS);
  child.on("exit", (code, signal) => {
    clearTimeout(timer);
    clearInterval(poll);
    const wallS = Number(process.hrtime.bigint() - started) / 1e9;
    writeFileSync(result, JSON.stringify({ exit: code ?? (signal === "SIGKILL" ? 137 : 128), killedBy, wallS, peakKB }));
  });
}

if (process.argv[1] !== undefined && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main();
