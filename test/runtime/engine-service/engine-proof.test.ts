import { spawn, type ChildProcess } from "node:child_process";
import { once } from "node:events";
import { existsSync } from "node:fs";
import { mkdir, mkdtemp, writeFile } from "node:fs/promises";
import { createServer } from "node:http";
import { createRequire } from "node:module";
import { connect, createServer as createSocketServer, type Socket } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createInterface } from "node:readline";
import { setTimeout as delay } from "node:timers/promises";
import { pathToFileURL } from "node:url";
import { afterEach, describe, expect, it, vi } from "vitest";
import { loadNativeBinding, LOCKS_DIR } from "../../../src/engine/index.js";
import { DAEMON_STATE_FILE, lookupDaemon, recordedDaemonStatus, recordedDaemonStatusForEvent, type DaemonState } from "../../../src/runtime/daemon-control.js";
import { engineCapabilityProof, engineHoldsCapability, engineProof } from "../../../src/runtime/daemon/engine-proof.js";
import { oneInstancePipeLines } from "../../support/platform/one-instance-pipe.js";
import { removeFolder } from "../../support/platform/remove-folder.js";

/** How far past its limit a proof may answer on a loaded test machine; the wait it must not take is 30 s. */
const SLACK_MS = 2_000;
/** How long a test waits for a process that should have ended once it answered, before it reads it as held. */
const HELD_MS = 25_000;

/**
 * A listener whose process's one event loop stops once it listens, as a daemon's does while it is paused or inside a
 * long audit: the OS still completes connections to what the listener set up before it stopped, and nothing answers.
 * On Windows Node sets up four instances of a pipe it listens on, so a fifth client finds none free.
 */
const STUCK_LISTENER = `
const server = require("node:net").createServer(() => {});
server.listen(process.argv[1], () => {
  process.stdout.write("listening\\n", () => Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 120000));
});`;

/**
 * The lines of a listener's script that take every instance of its own pipe, opened as a client, into `taken`, and
 * print how many it took and how the open after the last one ended: `busy <n> EBUSY` once none was free.
 */
const TAKE_EVERY_INSTANCE = `
  const taken = [];
  let ended = "free";
  while (taken.length < 16) {
    try { taken.push(openSync(path, "r+")); } catch (error) { if (error.code !== "EBUSY") throw error; ended = "EBUSY"; break; }
  }
  writeSync(1, "busy " + taken.length + " " + ended + "\\n");`;

/** The lines of an engine's script that answer proofs for "capability" on `path`, as `server`. */
const PROVING_SERVER = `
const { engineProof, ENGINE_CHALLENGE_HEADER } = await import(process.argv[1]);
const { closeSync, openSync, readSync, writeSync } = await import("node:fs");
const path = process.argv[2];
const server = (await import("node:http")).createServer((request, response) => {
  response.end(JSON.stringify({ proof: engineProof("capability", String(request.headers[ENGINE_CHALLENGE_HEADER])) }));
});`;

/**
 * An engine that answers proofs for "capability", whose process takes every instance of its own pipe and then stops its
 * event loop for the milliseconds it is given, as a busy daemon's does while its pipe has no free instance: once its
 * loop runs again it accepts those connections and sets up new instances, and a proof that asks again is answered.
 */
const BUSY_ENGINE = `${PROVING_SERVER}
server.listen(path, () => {${TAKE_EVERY_INSTANCE}
  Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, Number(process.argv[3]));
  for (const fd of taken) closeSync(fd);
});`;

/**
 * {@link BUSY_ENGINE} whose event loop stays stopped until the test writes to its input. Once its loop runs again, Node
 * accepts each connection it held and sets up a new instance of the pipe before it hands the connection over: the
 * engine prints `free` at the first, its own word that an instance is free. Given milliseconds, it answers the first
 * proof that long after the proof's request arrives, with its loop running, so the answer's delay is counted from the
 * proof's own connection, whichever instance that took: a proof's check of a busy pipe opens a freed instance and
 * closes it at once, and an engine whose loop stopped right after freeing one would take no new connection until it
 * ran again.
 */
const HELD_ENGINE = `${PROVING_SERVER}
const answerHold = Number(process.argv[3] ?? 0);
if (answerHold > 0) {
  const answer = server.listeners("request")[0];
  server.removeAllListeners("request");
  server.once("request", (request, response) => {
    server.on("request", answer);
    setTimeout(() => answer(request, response), answerHold);
  });
}
server.listen(path, () => {${TAKE_EVERY_INSTANCE}
  readSync(0, Buffer.alloc(1));
  server.once("connection", () => writeSync(1, "free\\n"));
  for (const fd of taken) closeSync(fd);
});`;

/**
 * A listener that takes every instance of its own pipe and ends once the test writes to its input, as a busy daemon
 * that is ended does: its pipe goes with its process.
 */
const HELD_LISTENER = `
const { openSync, readSync, writeSync } = require("node:fs");
const path = process.argv[1];
require("node:net").createServer(() => {}).listen(path, () => {${TAKE_EVERY_INSTANCE}
  readSync(0, Buffer.alloc(1));
  process.exit(0);
});`;

/** Windows' timer tick: a timer there fires only on one, so a wait of 1 to 15 ms takes about 15.6 ms. */
const TIMER_TICK_MS = 15.625;
/** A limit no case here reaches: a proof held by its test is never cut short by its own deadline. */
const HELD_LIMIT_MS = 30_000;
/** How many tries a held case lets the proof make while every instance of the pipe stays taken. */
const BUSY_TRIES = 3;

/** A process that asks the engine on a pipe for one proof with a 1 s limit, prints the answer, and ends by itself. */
const PROBER = `
const { engineCapabilityProof } = await import(process.argv[1]);
process.stdout.write(JSON.stringify({ answer: await engineCapabilityProof(process.argv[2], "capability", 1000) }) + "\\n");`;
/**
 * A process that asks the engine on a pipe for one proof with a 2 s limit, and prints the answer, how many tries the
 * proof made (one more than the waits it asked for) and how many folders it read meanwhile: `readdirSync` is counted
 * before the proof's module loads, and Node hands the counted one to every module that imports it.
 */
const TRY_PROBER = `
const fs = await import("node:fs");
const read = fs.default.readdirSync;
let counting = false;
let folderReads = 0;
fs.default.readdirSync = (...args) => { if (counting) folderReads += 1; return read(...args); };
(await import("node:module")).syncBuiltinESMExports();
const { engineCapabilityProof } = await import(process.argv[1]);
const { setTimeout: pause } = await import("node:timers/promises");
let tries = 1;
counting = true;
const answer = await engineCapabilityProof(process.argv[2], "capability", 2000, { sleep: async (ms) => { tries += 1; await pause(ms); } });
counting = false;
process.stdout.write(JSON.stringify({ answer, tries, folderReads }) + "\\n");`;
const PROOF_MODULE = new URL("../../../src/runtime/daemon/engine-proof.ts", import.meta.url).href;
const TSX = pathToFileURL(createRequire(import.meta.url).resolve("tsx")).href;

const children: ChildProcess[] = [];
const sockets: Socket[] = [];
const folders: string[] = [];
/** What a test opened in this process, closed after it however it ended. */
const closers: Array<() => void> = [];

afterEach(async () => {
  vi.useRealTimers();
  for (const close of closers.splice(0)) close();
  for (const socket of sockets.splice(0)) socket.destroy();
  await Promise.all(children.splice(0).map(async (child) => {
    if (child.exitCode !== null || child.signalCode !== null) return;
    const closed = once(child, "close");
    child.kill("SIGKILL");
    await closed;
  }));
  await Promise.all(folders.splice(0).map(async folder => await removeFolder(folder)));
});

/** The name, as `\\.\pipe\` lists it, of a named pipe no other test or run uses. */
function pipeName(name: string): string {
  return `rerail-proof-${name}-${process.pid}-${Date.now().toString(16)}`;
}

/** A named pipe no other test or run uses. */
function pipePath(name: string): string {
  return String.raw`\\.\pipe` + "\\" + pipeName(name);
}

/** The first output `child` prints; a child that ends first fails the test with its exit code and signal. */
async function firstLine(child: ChildProcess): Promise<string> {
  const printed = once(child.stdout!, "data").then(([chunk]: unknown[]) => ({ line: String(chunk) }));
  const ended = once(child, "close").then(([code, signal]: unknown[]) => ({ ended: `exit ${String(code)}, signal ${String(signal)}` }));
  const first = await Promise.race([printed, ended]);
  if ("ended" in first) throw new Error(`the child process ended (${first.ended}) before it printed anything`);
  return first.line;
}

/**
 * Starts `script` under Node with `args`, its output read by the test and, with `input`, its input written by it; `tsx`
 * loads the TypeScript it imports.
 */
function startNode(script: string, args: readonly string[], typescript = false, input = false): ChildProcess {
  const loader = typescript ? ["--import", TSX, "--input-type=module"] : [];
  const child = spawn(process.execPath, [...loader, "-e", script, ...args], { stdio: [input ? "pipe" : "ignore", "pipe", "inherit"], windowsHide: true });
  children.push(child);
  return child;
}

/** Starts {@link STUCK_LISTENER} on `path`, and returns once it listens and its event loop has stopped. */
async function stuckListener(path: string): Promise<ChildProcess> {
  const child = startNode(STUCK_LISTENER, [path]);
  expect(await firstLine(child)).toBe("listening\n");
  return child;
}

/** The lines `child` prints, one per call; a child that ends first fails the test. */
function linesOf(child: ChildProcess): () => Promise<string> {
  const lines = createInterface({ input: child.stdout! })[Symbol.asyncIterator]();
  return async () => {
    const next = await lines.next();
    if (next.done === true) throw new Error(`the child process ended (exit ${String(child.exitCode)}) before it printed a line`);
    return String(next.value);
  };
}

/**
 * Starts {@link BUSY_ENGINE} on `path`, and returns once every instance of its pipe is taken and its loop has stopped: a
 * proof then finds no free instance, and is answered only through its retry.
 */
async function busyEngine(path: string, stopMs: number): Promise<void> {
  expect(await firstLine(startNode(BUSY_ENGINE, [PROOF_MODULE, path, String(stopMs)], true))).toMatch(/^busy [1-9]\d* EBUSY\n$/u);
}

/**
 * The waits a proof asks for between its tries of a busy pipe, each held until the test ends it: `asked` holds how long
 * each was to last, and `next()` gives the function that ends the next one once the proof has asked for it.
 */
interface HeldWaits {
  asked: number[];
  sleep: (ms: number) => Promise<void>;
  next: () => Promise<() => void>;
}

function heldWaits(): HeldWaits {
  const asked: number[] = [];
  const turns: Array<PromiseWithResolvers<() => void>> = [];
  const turn = (index: number): PromiseWithResolvers<() => void> => (turns[index] ??= Promise.withResolvers<() => void>());
  let taken = 0;
  return {
    asked,
    sleep: async (ms) => {
      const { promise, resolve } = Promise.withResolvers<undefined>();
      turn(asked.length).resolve(() => resolve(undefined));
      asked.push(ms);
      await promise;
    },
    next: async () => await turn(taken++).promise,
  };
}

/**
 * Lets the proof `waits` holds try the busy pipe {@link BUSY_TRIES} times, each try finding every instance taken since
 * the proof asks for a wait only after such a try, and returns the end of the wait it asked for after the last.
 */
async function letTryWhileBusy(waits: HeldWaits): Promise<() => void> {
  for (let tried = 0; tried < BUSY_TRIES; tried += 1) (await waits.next())();
  return await waits.next();
}

/** The proof's answer, or `waited again` when it asks for another wait first: its try after the wait found no instance. */
async function answerOrWait(answered: Promise<string>, waits: HeldWaits): Promise<string> {
  return await Promise.race([answered, waits.next().then(() => "waited again")]);
}

/**
 * A holder of the pipe `name` that is not an engine, as any local user can create an engine pipe's name first: one
 * instance with no input buffer, which takes one client and never reads, so a write to it waits until the holder ends,
 * which it does once the test ends. With an `answer`, it writes that to the client first. Returns once the instance
 * exists. Its pipe is made as `oneInstancePipeLines` makes it, through .NET calls alone, so the case's wait for
 * `listening` waits on no module.
 */
async function holderThatNeverReads(name: string, answer?: string): Promise<void> {
  const script = [
    ...oneInstancePipeLines(name, { options: "None", outBufferSize: 4096 }),
    "[Console]::Out.WriteLine('listening'); [Console]::Out.Flush()",
    "$pipe.WaitForConnection()",
    ...(answer === undefined ? [] : [
      `$answer = [Convert]::FromBase64String('${Buffer.from(answer, "latin1").toString("base64")}')`,
      "$pipe.Write($answer, 0, $answer.Length); $pipe.Flush()",
    ]),
    // Lives until its input closes: the test's end, or the kill after it.
    "[void][Console]::In.ReadLine()",
  ].join("\n");
  const holder = spawn("powershell.exe", ["-NoProfile", "-NonInteractive", "-EncodedCommand", Buffer.from(script, "utf16le").toString("base64")], {
    stdio: ["pipe", "pipe", "inherit"], windowsHide: true,
  });
  children.push(holder);
  expect(await firstLine(holder)).toMatch(/^listening/u);
}

/**
 * Runs {@link PROBER} against `path`: its answer, and how long its process ran on after it answered, up to
 * {@link HELD_MS}.
 */
async function proveInAnotherProcess(path: string): Promise<{ answer: unknown; ranOnMs: number }> {
  const prober = startNode(PROBER, [PROOF_MODULE, path], true);
  const closed = once(prober, "close");
  const line = await firstLine(prober);
  const answeredAt = Date.now();
  await Promise.race([closed, delay(HELD_MS)]);
  return { answer: (JSON.parse(line) as { answer: unknown }).answer, ranOnMs: Date.now() - answeredAt };
}

/**
 * Connects to `path` until a connect does not complete within a second, and returns how many did: the stuck listener's
 * pipe instances are then all taken, and none comes back while its event loop stays stopped.
 */
async function takeEveryInstance(path: string): Promise<number> {
  for (let taken = 0; taken < 16; taken += 1) {
    const socket = connect(path);
    socket.on("error", () => undefined);
    sockets.push(socket);
    const connected = await Promise.race([once(socket, "connect").then(() => true), delay(1_000).then(() => false)]);
    if (!connected) return taken;
  }
  throw new Error(`${path} still had a free instance after 16 connections`);
}

function daemonState(pid: number, kernelSocketPath: string): DaemonState {
  return {
    pid,
    host: "127.0.0.1",
    port: 1,
    url: "http://127.0.0.1:1/",
    version: "0.0.0-test",
    databasePath: join(tmpdir(), "rerail.sqlite"),
    kernelSocketPath,
    ui: "dev",
    startedAt: 0,
    packageRoot: tmpdir(),
    accessToken: "capability",
  };
}

// Windows: once a stuck engine's pipe instances are taken, a connect waits for a free one for up to 30 s, and Node
// starts a request's own timeout only once its socket has connected.
describe.runIf(process.platform === "win32")("the engine proof, against an engine whose process is stuck", () => {
  it("answers silent within its limit, also once the engine's pipe has no free instance", async () => {
    const path = pipePath("stuck");
    await stuckListener(path);
    const began = Date.now();
    const answers = await Promise.all(Array.from({ length: 6 }, async () => {
      const answer = await engineCapabilityProof(path, "capability", 1_000);
      return { answer, ms: Date.now() - began };
    }));
    // Each answer that is not silence within the limit, with when it came.
    expect(answers.filter(({ answer, ms }) => answer !== "silent" || ms > 1_000 + SLACK_MS)).toEqual([]);
  }, 45_000);

  // The daemon here is this process, which holds the home's lock and records itself beside it, as a daemon does, while
  // the stuck listener holds the recorded pipe.
  it("leaves a lookup the state of a daemon whose engine pipe has no free instance while it holds the home, within its limit", async () => {
    const path = pipePath("lookup");
    await stuckListener(path);
    expect(await takeEveryInstance(path)).toBeGreaterThan(0);
    const home = await mkdtemp(join(tmpdir(), "rerail-proof-home-"));
    folders.push(home);
    const stateFile = join(home, DAEMON_STATE_FILE);
    const state = daemonState(process.pid, path);
    await writeFile(stateFile, JSON.stringify(state));
    await mkdir(join(home, LOCKS_DIR), { recursive: true });
    const lock = loadNativeBinding().tryLockExclusive(join(home, LOCKS_DIR, "daemon.lock"));
    expect(lock).not.toBeNull();
    await writeFile(join(home, LOCKS_DIR, "daemon.json"), JSON.stringify({ pid: process.pid }));
    let began = Date.now();
    const kept = await lookupDaemon(home);
    let ms = Date.now() - began;
    expect({ kept, file: existsSync(stateFile) }, `the lookup answered after ${ms} ms`).toEqual({ kept: { status: "unresponsive", state }, file: true });
    expect(ms).toBeLessThan(2_000 + SLACK_MS);
    // Once no daemon holds the home, the lookup has used its limit on the pipe, and still reads when this process
    // started, a system call that needs none: it started after the record's startedAt, so it is no daemon of the home,
    // and the record goes within the limit, with nothing sent.
    lock!.release();
    began = Date.now();
    const read = await lookupDaemon(home);
    ms = Date.now() - began;
    expect({ read, file: existsSync(stateFile) }, `the lookup answered after ${ms} ms`).toEqual({ read: { status: "absent", notDaemon: process.pid }, file: false });
    expect(ms).toBeLessThan(2_000 + SLACK_MS);
  }, 45_000);

  // A connect that waits for a free instance runs in a thread that closing the socket does not stop, and it keeps the
  // process alive for up to 30 s after the answer: a command or hook that asked would end that much later.
  it("leaves its process free to end once it has answered, also when the engine's pipe has no free instance", async () => {
    const path = pipePath("exit");
    await stuckListener(path);
    expect(await takeEveryInstance(path)).toBeGreaterThan(0);
    const { answer, ranOnMs } = await proveInAnotherProcess(path);
    expect(answer, `the process ended ${ranOnMs} ms after its answer`).toBe("silent");
    expect(ranOnMs).toBeLessThan(SLACK_MS);
  }, 45_000);

  // A try on every tick is a connect that fails at once and a plain open. Measured on Windows 11 with 455 pipes, over a
  // 2 s limit, by the event loop's own clock: 130 such tries took 37 to 43 ms, and a try on every tick that also listed
  // the machine's pipes took 135 to 155 ms. So the cost rests on what a try does, not on how fast the machine runs it:
  // a try reads no folder.
  it("reads no folder in any of its tries while every instance stays taken for the whole limit", async () => {
    const path = pipePath("cost");
    await stuckListener(path);
    expect(await takeEveryInstance(path)).toBeGreaterThan(0);
    const printed = JSON.parse(await firstLine(startNode(TRY_PROBER, [PROOF_MODULE, path], true))) as { answer: unknown; tries: number; folderReads: number };
    expect({ answer: printed.answer, folderReads: printed.folderReads }, `over ${printed.tries} tries`).toEqual({ answer: "silent", folderReads: 0 });
    expect(printed.tries, "the proof's tries").toBeGreaterThan(1);
  }, 45_000);
});

describe.runIf(process.platform === "win32")("the engine proof, against a pipe's holder that takes the request and never reads it", () => {
  // A write the holder never reads waits for as long as the holder lives, and a command or hook that asked would wait
  // with it.
  it("leaves its process free to end once it has answered", async () => {
    const name = pipeName("unread");
    await holderThatNeverReads(name);
    const { answer, ranOnMs } = await proveInAnotherProcess(String.raw`\\.\pipe` + "\\" + name);
    expect(answer, `the process ended ${ranOnMs} ms after its answer`).toBe("silent");
    expect(ranOnMs).toBeLessThan(SLACK_MS);
  }, 45_000);

  // A holder that answers at once leaves the request's write pending: only destroying the request at the verdict
  // cancels it.
  it("leaves its process free to end once it has a verdict, also when the holder answers before it reads the request", async () => {
    const name = pipeName("answered");
    await holderThatNeverReads(name, "HTTP/1.1 403 Forbidden\r\ncontent-length: 0\r\n\r\n");
    const { answer, ranOnMs } = await proveInAnotherProcess(String.raw`\\.\pipe` + "\\" + name);
    expect(answer, `the process ended ${ranOnMs} ms after its answer`).toBe("refused");
    expect(ranOnMs).toBeLessThan(SLACK_MS);
  }, 45_000);
});

describe.runIf(process.platform === "win32")("the engine proof, against a busy engine whose pipe has no free instance for a while", () => {
  // Node sets up four instances of a pipe it listens on: a burst of hooks finds them all taken for a moment.
  it("is answered once the engine frees an instance within the limit", async () => {
    const path = pipePath("burst");
    await busyEngine(path, 500);
    const began = Date.now();
    expect(await engineCapabilityProof(path, "capability", 2_000), `the proof answered after ${Date.now() - began} ms`).toBe("proved");
  }, 45_000);

  // main's connect waited for a free instance inside Windows, which hands it one the moment the engine sets it up. A
  // proof that waits between tries finds the instance at its next try, so it is answered within a timer tick of the
  // engine freeing one when the try after the engine's word is the one answered and each wait it asks for ends on the
  // next tick. The engine and the proof's waits move only when the test lets them, so a slow machine changes neither.
  it("is answered at its first try after the engine frees an instance, each wait between tries shorter than a timer tick", async () => {
    const path = pipePath("freed");
    const engine = startNode(HELD_ENGINE, [PROOF_MODULE, path], true, true);
    const line = linesOf(engine);
    expect(await line()).toMatch(/^busy [1-9]\d* EBUSY$/u);
    const waits = heldWaits();
    const answered = engineCapabilityProof(path, "capability", HELD_LIMIT_MS, { sleep: waits.sleep });
    const endLastWait = await letTryWhileBusy(waits);
    engine.stdin!.write("go\n");
    expect(await line()).toBe("free");
    endLastWait();
    expect(await answerOrWait(answered, waits)).toBe("proved");
    expect(waits.asked, "the waits the proof asked for, in ms").toHaveLength(BUSY_TRIES + 1);
    expect(waits.asked.filter(ms => ms >= TIMER_TICK_MS), "the waits of a timer tick or longer, in ms").toEqual([]);
  }, 45_000);

  // main's wait for a free instance ended as soon as the pipe went, and the lookup then forgot the ended daemon.
  it("answers refused at its first try after the engine's pipe goes, each wait between tries shorter than a timer tick", async () => {
    const path = pipePath("gone");
    const listener = startNode(HELD_LISTENER, [path], false, true);
    expect(await firstLine(listener)).toMatch(/^busy [1-9]\d* EBUSY\n$/u);
    const waits = heldWaits();
    const answered = engineCapabilityProof(path, "capability", HELD_LIMIT_MS, { sleep: waits.sleep });
    const endLastWait = await letTryWhileBusy(waits);
    // Windows closes a process's handles before it reports the process ended: the pipe has gone by then.
    const ended = once(listener, "exit");
    listener.stdin!.write("go\n");
    expect(await ended).toEqual([0, null]);
    endLastWait();
    expect(await answerOrWait(answered, waits)).toBe("refused");
    expect(waits.asked, "the waits the proof asked for, in ms").toHaveLength(BUSY_TRIES + 1);
    expect(waits.asked.filter(ms => ms >= TIMER_TICK_MS), "the waits of a timer tick or longer, in ms").toEqual([]);
  }, 45_000);

  // main's proof connected as Node does, which waits up to 30 s for a free instance before the proof's own limit began:
  // a hook's event reached a busy daemon that freed one in that time. Both proofs ask while every instance is taken; the
  // clock they read then moves past a command's 2 s and stays inside the 30 s, and only then is an instance freed.
  it("waits past its limit for a free instance when a hook asks, as main's connect did, where a command's proof answers silent", async () => {
    const path = pipePath("instance");
    const engine = startNode(HELD_ENGINE, [PROOF_MODULE, path], true, true);
    const line = linesOf(engine);
    expect(await line()).toMatch(/^busy [1-9]\d* EBUSY$/u);
    vi.useFakeTimers({ toFake: ["Date"] });
    const command = engineCapabilityProof(path, "capability", 2_000);
    const event = engineCapabilityProof(path, "capability", 2_000, { waitForInstance: true });
    vi.setSystemTime(Date.now() + 10_000);
    expect(await command).toBe("silent");
    engine.stdin!.write("go\n");
    expect(await line()).toBe("free");
    // The engine then has the proof's 2 s from the connect, not from the ask.
    expect(await event).toBe("proved");
  }, 45_000);

  // A command keeps its 2 s from the ask however late its connect comes: here every instance stays taken until 1.9 s
  // after the ask, and the engine answers the proof 1 s after its request arrives, past the command's 2 s. The
  // command's limit then has 0.1 s left, and a limit counted from the connect would have 2 s: the engine's answer
  // falls 0.9 s past the one and 1 s inside the other, whenever the proof connects. The proof asked next is answered
  // at once, so the first proof's silence is its limit's.
  it("answers a command silent 2 s from its ask when an instance frees 1.9 s in to an engine that answers 1 s later", async () => {
    const path = pipePath("late");
    const engine = startNode(HELD_ENGINE, [PROOF_MODULE, path, "1000"], true, true);
    const line = linesOf(engine);
    expect(await line()).toMatch(/^busy [1-9]\d* EBUSY$/u);
    vi.useFakeTimers({ toFake: ["Date"] });
    const command = engineCapabilityProof(path, "capability", 2_000);
    vi.setSystemTime(Date.now() + 1_900);
    engine.stdin!.write("go\n");
    expect(await line()).toBe("free");
    expect(await command).toBe("silent");
    expect(await engineCapabilityProof(path, "capability", 2_000), "a proof asked next").toBe("proved");
  }, 45_000);

  // The same through what a hook's start check asks of the daemon daemon.json records, here this process with the
  // engine's pipe: the daemon is alive once it frees an instance, where a command's read finds it silent.
  it("lets a hook's read of the recorded daemon find it alive once its busy pipe frees an instance, where a command's read finds it silent", async () => {
    const path = pipePath("hook-read");
    const engine = startNode(HELD_ENGINE, [PROOF_MODULE, path], true, true);
    const line = linesOf(engine);
    expect(await line()).toMatch(/^busy [1-9]\d* EBUSY$/u);
    const state = daemonState(process.pid, path);
    vi.useFakeTimers({ toFake: ["Date"] });
    const command = recordedDaemonStatus(state);
    const event = recordedDaemonStatusForEvent(state);
    vi.setSystemTime(Date.now() + 10_000);
    expect(await command).toBe("silent");
    engine.stdin!.write("go\n");
    expect(await line()).toBe("free");
    expect(await event).toBe("alive");
  }, 45_000);

  // The engine start asks before it listens beside a held pipe: a busy engine of this home read as another process
  // would get a second daemon beside it. Within the 15 s `rerail daemon start` waits for the daemon, such an engine
  // looks the same as a holder that keeps every instance taken, so the start waits for a free instance past them.
  it("lets the engine start find a busy engine of this home that frees an instance only after the daemon start's 15 s", async () => {
    const path = pipePath("start");
    await busyEngine(path, 20_000);
    const began = Date.now();
    expect(await engineHoldsCapability(path, "capability"), `the proof answered after ${Date.now() - began} ms`).toBe(true);
  }, 45_000);
});

describe("the engine proof's limit", () => {
  // The limit bounds the whole exchange. A request's own timeout counts idle time, which a listener that keeps writing
  // a little of an answer resets for as long as it writes.
  it("answers silent at its limit when the listener keeps writing an answer that never ends", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-proof-"));
    folders.push(root);
    const path = process.platform === "win32" ? pipePath("trickle") : join(root, "engine.sock");
    const writers: NodeJS.Timeout[] = [];
    const listener = createServer((_request, response) => {
      response.writeHead(200, { "content-type": "application/json" });
      writers.push(setInterval(() => response.write(" "), 100));
    });
    closers.push(() => {
      for (const writer of writers) clearInterval(writer);
      listener.closeAllConnections();
      listener.close();
    });
    listener.listen(path);
    await once(listener, "listening");
    const began = Date.now();
    const answer = await engineCapabilityProof(path, "capability", 1_000);
    const ms = Date.now() - began;
    expect(answer, `the proof answered after ${ms} ms`).toBe("silent");
    expect(ms).toBeLessThan(1_000 + SLACK_MS);
  }, 15_000);

  // Once the proof has its verdict nothing of the exchange is left: a listener that answers and keeps the connection
  // open must not keep a command or hook that asked.
  it("leaves its process free to end once it has a verdict, also when the listener keeps the connection open", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-proof-"));
    folders.push(root);
    const path = process.platform === "win32" ? pipePath("open") : join(root, "engine.sock");
    const listener = createSocketServer((socket) => {
      sockets.push(socket);
      let asked = "";
      socket.on("data", (chunk: Buffer) => {
        asked += chunk.toString("latin1");
        const challenge = /x-rerail-challenge: ([0-9a-f]{64})/iu.exec(asked)?.[1];
        if (challenge === undefined || !asked.includes("\r\n\r\n")) return;
        const body = JSON.stringify({ proof: engineProof("capability", challenge) });
        socket.write(`HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: ${body.length}\r\n\r\n${body}`);
      });
    });
    closers.push(() => listener.close());
    listener.listen(path);
    await once(listener, "listening");
    const { answer, ranOnMs } = await proveInAnotherProcess(path);
    expect(answer, `the process ended ${ranOnMs} ms after its answer`).toBe("proved");
    expect(ranOnMs).toBeLessThan(SLACK_MS);
  }, 45_000);
});

describe("the engine proof's request", () => {
  // An engine reads the request a proof sends as main sent it. Node writes its own Host header with a port when a
  // request has a connection of its own and no agent.
  it("is the request main sent, on a Windows pipe and on a Unix socket", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-proof-"));
    folders.push(root);
    const path = process.platform === "win32" ? pipePath("request") : join(root, "engine.sock");
    let asked = "";
    const listener = createSocketServer((socket) => {
      sockets.push(socket);
      socket.on("data", (chunk: Buffer) => {
        asked += chunk.toString("latin1");
        if (asked.includes("\r\n\r\n")) socket.end("HTTP/1.1 403 Forbidden\r\ncontent-length: 0\r\n\r\n");
      });
    });
    closers.push(() => listener.close());
    listener.listen(path);
    await once(listener, "listening");
    const challenge = "0123456789abcdef".repeat(4);
    expect(await engineCapabilityProof(path, "capability", 1_000, { challenge })).toBe("refused");
    expect(asked).toBe(`POST /proof HTTP/1.1\r\nx-rerail-challenge: ${challenge}\r\ncontent-length: 0\r\nHost: localhost\r\nConnection: close\r\n\r\n`);
  }, 15_000);
});

describe("the engine proof, against an engine of a release before it", () => {
  /**
   * Listens on a new endpoint as the engine of Rerail 0.1.4 and earlier does, which has no proof: it takes `POST /events`
   * only with the capability and answers every other path 404. `proofStatus` replaces that 404 on the proof's path, for a
   * listener that refuses the proof otherwise. Resolves to the endpoint.
   */
  async function engineWithoutProof(name: string, proofStatus = 404): Promise<string> {
    const root = await mkdtemp(join(tmpdir(), "rerail-proof-"));
    folders.push(root);
    const path = process.platform === "win32" ? pipePath(name) : join(root, "engine.sock");
    const listener = createServer((request, response) => {
      request.resume();
      response.setHeader("Content-Type", "application/json");
      if (request.method === "POST" && request.url === "/events") response.writeHead(401).end(JSON.stringify({ error: "engine-access-required" }));
      else response.writeHead(request.url === "/proof" ? proofStatus : 404).end();
    });
    closers.push(() => {
      listener.closeAllConnections();
      listener.close();
    });
    listener.listen(path);
    await once(listener, "listening");
    return path;
  }

  // A 404 on the proof's path is the answer of an engine built before the proof, and of any listener without that path:
  // it proves the listener neither this home's engine nor another's. The engine start has no record to weigh it against.
  it("answers no-proof to a 404 on the proof's path, which the engine start does not take for its home's engine", async () => {
    const path = await engineWithoutProof("older");
    expect(await engineCapabilityProof(path, "capability", 1_000)).toBe("no-proof");
    expect(await engineHoldsCapability(path, "capability", 1_000)).toBe(false);
  }, 15_000);

  it.each([401, 403, 500])("answers refused to a %i on the proof's path", async (status) => {
    const path = await engineWithoutProof(`refusing-${status}`, status);
    expect(await engineCapabilityProof(path, "capability", 1_000)).toBe("refused");
  }, 15_000);
});
