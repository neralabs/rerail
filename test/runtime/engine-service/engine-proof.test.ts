import { spawn, type ChildProcess } from "node:child_process";
import { once } from "node:events";
import { existsSync } from "node:fs";
import { mkdtemp, writeFile } from "node:fs/promises";
import { createServer } from "node:http";
import { connect, type Socket } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { afterEach, describe, expect, it } from "vitest";
import { DAEMON_STATE_FILE, lookupDaemon, type DaemonState } from "../../../src/runtime/daemon-control.js";
import { engineCapabilityProof } from "../../../src/runtime/engine-proof.js";
import { removeFolder } from "../../support/platform/remove-folder.js";

/** How far past its limit a proof may answer on a loaded test machine; the wait it must not take is 30 s. */
const SLACK_MS = 2_000;

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

/** A named pipe no other test or run uses. */
function pipePath(name: string): string {
  return String.raw`\\.\pipe\rerail-proof-${name}-${process.pid}-${Date.now().toString(16)}`;
}

/** Starts {@link STUCK_LISTENER} on `path`, and returns once it listens and its event loop has stopped. */
async function stuckListener(path: string): Promise<ChildProcess> {
  const child = spawn(process.execPath, ["-e", STUCK_LISTENER, path], { stdio: ["ignore", "pipe", "inherit"], windowsHide: true });
  children.push(child);
  const [line] = await once(child.stdout!, "data") as [Buffer];
  expect(String(line)).toBe("listening\n");
  return child;
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
});
