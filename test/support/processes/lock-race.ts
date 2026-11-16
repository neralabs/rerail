import { spawn, type ChildProcess } from "node:child_process";
import { once } from "node:events";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";

/**
 * Many processes taking one lock at the moment its holder is killed, as sessions starting the daemon, set-ups and
 * updates on one machine do. Each round one process takes the lock and is killed while it holds it; then every racer
 * takes the lock at once. A racer tells this process when it is inside and waits for the answer before it lets go, so
 * this process sees every entry and exit in the order they happened: an overlap is a racer entering while another is
 * inside.
 */
export interface LockRace {
  /**
   * ES module source that defines `take(inside, role)`: takes the lock, runs `inside()` holding it, and lets go. A
   * racer finding the lock held either waits or throws, as the lock's own users do. `role` is "racer", or "killed" for
   * the process killed holding it.
   */
  readonly take: string;
  readonly processes: number;
  readonly rounds: number;
  /** A refusal that says another process holds the lock (`rerail update` does not wait), rather than a failure. */
  readonly busy?: RegExp;
  /** Added to every racer's environment. */
  readonly env?: Readonly<Record<string, string>>;
  /** How long a racer stays inside. */
  readonly holdMs?: number;
}

export interface LockRaceResult {
  /** Entries while another racer was inside. */
  overlaps: number;
  /** Takes that ran inside the lock. */
  ran: number;
  /** Takes refused as `busy` says. */
  busy: number;
  /** Every other failure, by message. */
  errors: string[];
}

/** The source modules a racer's `take` imports, as file URLs. */
export const sourceModule = (path: string): string => pathToFileURL(resolve(import.meta.dirname, "..", "..", "..", path)).href;

/**
 * The race's size: `RERAIL_LOCK_RACE=<processes>x<rounds>` (`16x40` is the stress these locks were measured with), else
 * `fallback`, small enough for the suite.
 */
export function raceSize(fallback: { processes: number; rounds: number }): { processes: number; rounds: number } {
  const match = /^(\d+)x(\d+)$/u.exec(process.env.RERAIL_LOCK_RACE ?? "");
  return match === null ? fallback : { processes: Number(match[1]), rounds: Number(match[2]) };
}

interface RacerMessage {
  kind: "ready" | "held" | "ran" | "failed" | "enter" | "leave";
  id?: number;
  message?: string;
}

const racerProgram = (take: string): string => `${take}
const holdMs = Number(process.env.RERAIL_RACE_HOLD_MS);
const answers = new Map();
let asked = 0;
const ask = kind => new Promise(resolve => { asked += 1; answers.set(asked, resolve); process.send({ kind, id: asked }); });
const failed = error => process.send({ kind: "failed", message: String(error?.message ?? error) });
process.on("message", message => {
  if (message.kind === "answer") {
    answers.get(message.id)();
    answers.delete(message.id);
  } else if (message.kind === "hold") {
    take(() => { process.send({ kind: "held" }); return new Promise(() => {}); }, "killed").catch(failed);
  } else if (message.kind === "go") {
    take(async () => {
      await ask("enter");
      await new Promise(resolve => setTimeout(resolve, holdMs));
      await ask("leave");
    }, "racer").then(() => process.send({ kind: "ran" }), failed);
  }
});
process.send({ kind: "ready" });
`;

interface Racer {
  readonly child: ChildProcess;
  stderr: string;
}

/** The next message of one of `kinds` from `racer`; rejects if it exits first. */
function next(racer: Racer, kinds: readonly RacerMessage["kind"][]): Promise<RacerMessage> {
  return new Promise((resolve, reject) => {
    const onMessage = (message: RacerMessage): void => {
      if (!kinds.includes(message.kind)) return;
      racer.child.off("exit", onExit);
      racer.child.off("message", onMessage);
      resolve(message);
    };
    const onExit = (code: number | null, signal: NodeJS.Signals | null): void => {
      racer.child.off("message", onMessage);
      reject(new Error(`a racer exited (${code ?? signal}) before it said ${kinds.join(" or ")}: ${racer.stderr}`));
    };
    racer.child.on("message", onMessage);
    racer.child.once("exit", onExit);
  });
}

async function startRacer(program: string, env: NodeJS.ProcessEnv): Promise<Racer> {
  const child = spawn(process.execPath, ["--import", "tsx", "--input-type=module", "-e", program], {
    cwd: resolve(import.meta.dirname, "..", "..", ".."),
    env,
    stdio: ["ignore", "ignore", "pipe", "ipc"],
    windowsHide: true,
  });
  const racer: Racer = { child, stderr: "" };
  child.stderr?.setEncoding("utf8").on("data", (chunk: string) => { racer.stderr = (racer.stderr + chunk).slice(-4_000); });
  await next(racer, ["ready"]);
  return racer;
}

async function kill(racer: Racer): Promise<void> {
  if (racer.child.exitCode !== null || racer.child.signalCode !== null) return;
  const exited = once(racer.child, "exit");
  racer.child.kill("SIGKILL");
  await exited;
}

/** Takes the lock in `victim` and kills it there, leaving the lock as a holder that died leaves it. */
async function dieHolding(victim: Racer): Promise<void> {
  const held = next(victim, ["held", "failed"]);
  victim.child.send({ kind: "hold" });
  const outcome = await held;
  if (outcome.kind === "failed") throw new Error(`the process to be killed holding the lock could not take it: ${outcome.message}`);
  await kill(victim);
}

export async function raceForLock(race: LockRace): Promise<LockRaceResult> {
  const program = racerProgram(race.take);
  const env = { ...process.env, ...race.env, RERAIL_RACE_HOLD_MS: String(race.holdMs ?? 20) };
  const result: LockRaceResult = { overlaps: 0, ran: 0, busy: 0, errors: [] };
  const started: Promise<Racer>[] = [];
  const start = (): Promise<Racer> => {
    const racer = startRacer(program, env);
    started.push(racer);
    return racer;
  };
  try {
    const racers = await Promise.all(Array.from({ length: race.processes }, start));
    let inside = 0;
    for (const { child } of racers) {
      child.on("message", (message: RacerMessage) => {
        if (message.kind !== "enter" && message.kind !== "leave") return;
        inside += message.kind === "enter" ? 1 : -1;
        if (message.kind === "enter" && inside > 1) result.overlaps += 1;
        child.send({ kind: "answer", id: message.id });
      });
    }
    let victim = start();
    for (let round = 0; round < race.rounds; round += 1) {
      const dying = await victim;
      // The next round's victim starts while this round runs.
      victim = start();
      await dieHolding(dying);
      const outcomes = await Promise.all(racers.map(racer => {
        const outcome = next(racer, ["ran", "failed"]);
        racer.child.send({ kind: "go" });
        return outcome;
      }));
      for (const outcome of outcomes) {
        if (outcome.kind === "ran") result.ran += 1;
        else if (race.busy?.test(outcome.message ?? "") === true) result.busy += 1;
        else result.errors.push(outcome.message ?? "");
      }
    }
  } finally {
    const settled = await Promise.allSettled(started);
    await Promise.all(settled.flatMap(outcome => outcome.status === "fulfilled" ? [kill(outcome.value)] : []));
  }
  return result;
}
