import { spawn, type ChildProcess } from "node:child_process";
import { once } from "node:events";
import { existsSync } from "node:fs";
import { mkdtemp, writeFile } from "node:fs/promises";
import { createServer } from "node:http";
import { createRequire } from "node:module";
import { connect, createServer as createSocketServer, type Socket } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { pathToFileURL } from "node:url";
import { afterEach, describe, expect, it } from "vitest";
import { DAEMON_STATE_FILE, lookupDaemon, type DaemonState } from "../../../src/runtime/daemon-control.js";
import { engineCapabilityProof, engineHoldsCapability, engineProof } from "../../../src/runtime/engine-proof.js";
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
 * An engine that answers proofs for "capability", whose process takes every instance of its own pipe and then stops its
 * event loop for the milliseconds it is given, as a busy daemon's does while its pipe has no free instance: once its
 * loop runs again it accepts those connections and sets up new instances, and a proof that asks again is answered.
 */
const BUSY_ENGINE = `
const { engineProof, ENGINE_CHALLENGE_HEADER } = await import(process.argv[1]);
const { openSync, writeSync } = await import("node:fs");
const [path, stopMs] = process.argv.slice(2);
const server = (await import("node:http")).createServer((request, response) => {
  response.end(JSON.stringify({ proof: engineProof("capability", String(request.headers[ENGINE_CHALLENGE_HEADER])) }));
});
server.listen(path, () => {
  const taken = [];
  while (taken.length < 16) {
    try { taken.push(openSync(path, "r+")); } catch { break; }
  }
  writeSync(1, "busy " + taken.length + "\\n");
  Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, Number(stopMs));
});`;

/** A process that asks the engine on a pipe for one proof with a 1 s limit, prints the answer, and ends by itself. */
const PROBER = `
const { engineCapabilityProof } = await import(process.argv[1]);
process.stdout.write(JSON.stringify({ answer: await engineCapabilityProof(process.argv[2], "capability", 1000) }) + "\\n");`;
const PROOF_MODULE = new URL("../../../src/runtime/engine-proof.ts", import.meta.url).href;
const TSX = pathToFileURL(createRequire(import.meta.url).resolve("tsx")).href;

const children: ChildProcess[] = [];
const sockets: Socket[] = [];
const folders: string[] = [];
/** What a test opened in this process, closed after it however it ended. */
const closers: Array<() => void> = [];

afterEach(async () => {
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

/** Starts `script` under Node with `args`, its output read by the test; `tsx` loads the TypeScript it imports. */
function startNode(script: string, args: readonly string[], typescript = false): ChildProcess {
  const loader = typescript ? ["--import", TSX, "--input-type=module"] : [];
  const child = spawn(process.execPath, [...loader, "-e", script, ...args], { stdio: ["ignore", "pipe", "inherit"], windowsHide: true });
  children.push(child);
  return child;
}

/** Starts {@link STUCK_LISTENER} on `path`, and returns once it listens and its event loop has stopped. */
async function stuckListener(path: string): Promise<ChildProcess> {
  const child = startNode(STUCK_LISTENER, [path]);
  expect(await firstLine(child)).toBe("listening\n");
  return child;
}

/** Starts {@link BUSY_ENGINE} on `path`, and returns once every instance of its pipe is taken and its loop has stopped. */
async function busyEngine(path: string, stopMs: number): Promise<void> {
  expect(await firstLine(startNode(BUSY_ENGINE, [PROOF_MODULE, path, String(stopMs)], true))).toMatch(/^busy [1-9]/u);
}

/**
 * A holder of the pipe `name` that is not an engine, as any local user can create an engine pipe's name first: one
 * instance with no input buffer, which takes one client and never reads, so a write to it waits until the holder ends,
 * 20 s on. Returns once the instance exists.
 */
async function holderThatNeverReads(name: string): Promise<void> {
  const script = [
    "$ErrorActionPreference = 'Stop'",
    "Add-Type -AssemblyName System.Core",
    `$pipe = New-Object System.IO.Pipes.NamedPipeServerStream('${name}', [System.IO.Pipes.PipeDirection]::InOut, 1, [System.IO.Pipes.PipeTransmissionMode]::Byte, [System.IO.Pipes.PipeOptions]::None, 0, 0)`,
    "[Console]::Out.WriteLine('listening'); [Console]::Out.Flush()",
    "$pipe.WaitForConnection()",
    "Start-Sleep -Seconds 20",
  ].join("\n");
  const holder = spawn("powershell.exe", ["-NoProfile", "-NonInteractive", "-EncodedCommand", Buffer.from(script, "utf16le").toString("base64")], {
    stdio: ["ignore", "pipe", "inherit"], windowsHide: true,
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

  it("leaves a lookup the state of a daemon whose engine pipe has no free instance, which it reads as not answering", async () => {
    const path = pipePath("lookup");
    const engine = await stuckListener(path);
    expect(await takeEveryInstance(path)).toBeGreaterThan(0);
    const home = await mkdtemp(join(tmpdir(), "rerail-proof-home-"));
    folders.push(home);
    const stateFile = join(home, DAEMON_STATE_FILE);
    const state = daemonState(engine.pid!, path);
    await writeFile(stateFile, JSON.stringify(state));
    const began = Date.now();
    const lookup = await lookupDaemon(home);
    const ms = Date.now() - began;
    expect({ lookup, kept: existsSync(stateFile) }, `the lookup answered after ${ms} ms`).toEqual({ lookup: { status: "unresponsive", state }, kept: true });
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
});

describe.runIf(process.platform === "win32")("the engine proof, against a busy engine whose pipe has no free instance for a while", () => {
  // Node sets up four instances of a pipe it listens on: a burst of hooks finds them all taken for a moment.
  it("is answered once the engine frees an instance within the limit", async () => {
    const path = pipePath("burst");
    await busyEngine(path, 500);
    const began = Date.now();
    expect(await engineCapabilityProof(path, "capability", 2_000), `the proof answered after ${Date.now() - began} ms`).toBe("proved");
  }, 45_000);

  // The engine start asks before it listens beside a held pipe: a busy engine of this home read as another process
  // would get a second daemon beside it.
  it("lets the engine start find a busy engine of this home that frees an instance after a lookup's limit", async () => {
    const path = pipePath("start");
    await busyEngine(path, 3_000);
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
