import { spawnSync } from "node:child_process";
import { once } from "node:events";
import { closeSync, existsSync, mkdirSync, openSync } from "node:fs";
import { chmod, lstat, mkdir, mkdtemp, realpath, symlink, writeFile } from "node:fs/promises";
import { ClientRequest, createServer as createHttpServer } from "node:http";
import { tmpdir } from "node:os";
import { connect, createServer, type Server, type Socket } from "node:net";
import { dirname, join } from "node:path";
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { ENGINE_ANSWER_WAIT_MS, engineEndpointAt } from "../../../src/runtime/daemon-control.js";
import { engineHoldsCapability, engineProof } from "../../../src/runtime/daemon/engine-proof.js";
import { enginePipeName, fallbackPipe, isEnginePipe, pipeNameOf, pipeServer } from "../../../src/runtime/daemon/engine-pipe.js";
import {
  chooseEngineSocketPath, ingestEngineEvent, ingestEngineEventAnswer, listenBesideHeldPipe, preparePrivateDirectory, startEngineService,
} from "../../../src/runtime/engine-service.js";
import type { HeldSession } from "../../../src/engine/index.js";
import { testEmbedder } from "../../support/hook-events.js";
import { startTestRepository } from "../../support/started-repository.js";
import { fakeMachine } from "../../support/platform/daemon-machine.js";
import { oneInstanceHolder } from "../../support/platform/one-instance-pipe.js";
import { removeFolder, removeFolderSync } from "../../support/platform/remove-folder.js";

// A TMPDIR longer than any socket path under it can be: a Unix socket path holds at most 107 bytes on Linux and 103 on
// macOS, whose own TMPDIR (`/var/folders/<2>/<28>/T`) is 48 characters. Every test below passes whatever its length,
// since each makes its Unix sockets under SOCKET_ROOT.
const SOCKET_ROOT = "/tmp";
const originalTmpdir = process.env.TMPDIR;
let longTmpdir: string | undefined;
beforeAll(() => {
  if (process.platform === "win32") return;
  longTmpdir = join(tmpdir(), "t".repeat(80));
  mkdirSync(longTmpdir, { recursive: true });
  process.env.TMPDIR = longTmpdir;
});
afterAll(() => {
  if (originalTmpdir === undefined) delete process.env.TMPDIR;
  else process.env.TMPDIR = originalTmpdir;
  if (longTmpdir !== undefined) removeFolderSync(longTmpdir);
});

describe.runIf(process.platform !== "win32")("engine service socket directory", () => {
  const roots: string[] = [];

  async function temporaryRoot(): Promise<string> {
    const root = await mkdtemp(join(SOCKET_ROOT, "rerail-socket-"));
    roots.push(root);
    return root;
  }

  afterEach(async () => {
    await Promise.all(roots.splice(0).map(root => removeFolder(root)));
  });

  it("listens inside a directory it makes for this user alone", async () => {
    const directory = join(await temporaryRoot(), "rerail-socket", "nested");
    const socketPath = join(directory, "engine.sock");
    const service = await startEngineService({ socketPath, storageRoot: join(directory, "..", "state") });
    try {
      const info = await lstat(directory);
      expect(info.isDirectory()).toBe(true);
      expect(info.uid).toBe(process.getuid!());
      expect(info.mode & 0o777).toBe(0o700);
      expect((await lstat(socketPath)).isSocket()).toBe(true);
    } finally {
      await service.close();
    }
  });

  it("clears the socket file a killed daemon left behind, and listens there", async () => {
    const directory = join(await temporaryRoot(), "run");
    await mkdir(directory, { mode: 0o700 });
    const socketPath = join(directory, "engine.sock");
    // A daemon killed with SIGKILL never closes its server, so its socket file outlives it.
    const killed = spawnSync(process.execPath, [
      "-e",
      "require('node:net').createServer().listen(process.argv[1], () => process.kill(process.pid, 'SIGKILL'))",
      socketPath,
    ]);
    expect(killed.signal).toBe("SIGKILL");
    expect((await lstat(socketPath)).isSocket()).toBe(true);
    const service = await startEngineService({ socketPath, storageRoot: join(directory, "state") });
    try {
      await expect(ingestEngineEvent({ socketPath }, { kind: "session-end", eventId: "end", sessionId: "s" })).rejects.toThrow(/\(400\)/u);
    } finally {
      await service.close();
    }
  });

  it("leaves a socket a live process answers on alone, and names it as a running daemon's", async () => {
    const directory = join(await temporaryRoot(), "run");
    await mkdir(directory, { mode: 0o700 });
    const socketPath = join(directory, "engine.sock");
    const live = createServer(socket => socket.end());
    await new Promise<void>(resolve => live.listen(socketPath, resolve));
    try {
      await expect(startEngineService({ socketPath, storageRoot: join(directory, "state") })).rejects.toThrow(/in use by a running daemon/u);
      expect((await lstat(socketPath)).isSocket()).toBe(true);
    } finally {
      await new Promise<void>(resolve => live.close(() => resolve()));
    }
  });

  it("leaves a file that is not a socket alone, and says so", async () => {
    const directory = join(await temporaryRoot(), "run");
    await mkdir(directory, { mode: 0o700 });
    const socketPath = join(directory, "engine.sock");
    await writeFile(socketPath, "not a socket");
    await expect(startEngineService({ socketPath, storageRoot: join(directory, "state") })).rejects.toThrow(/not a socket/u);
    expect((await lstat(socketPath)).isFile()).toBe(true);
  });

  it("refuses a directory that others may write, and listens nowhere", async () => {
    const directory = join(await temporaryRoot(), "shared");
    await mkdir(directory);
    await chmod(directory, 0o777);
    const socketPath = join(directory, "engine.sock");
    await expect(startEngineService({ socketPath, storageRoot: join(directory, "state") })).rejects.toThrow(/has mode 777, not 700/u);
    expect(existsSync(socketPath)).toBe(false);
  });

  it("refuses a directory another user owns, and a link standing in for a directory", async () => {
    const root = await temporaryRoot();
    const directory = join(root, "owned");
    await mkdir(directory, { mode: 0o700 });
    await expect(preparePrivateDirectory(directory, process.getuid!() + 1)).rejects.toThrow(/is owned by uid/u);
    await expect(preparePrivateDirectory(directory)).resolves.toBeUndefined();

    const link = join(root, "link");
    await symlink(directory, link);
    const socketPath = join(link, "engine.sock");
    await expect(startEngineService({ socketPath, storageRoot: join(root, "state") })).rejects.toThrow(/is a link or not a directory/u);
    expect(existsSync(join(directory, "engine.sock"))).toBe(false);
  });

  it("refuses an event without its capability before anything is written where the event asks", async () => {
    const root = await temporaryRoot();
    startTestRepository(root, join(root, "rerail.sqlite"));
    const socketPath = join(root, "socket", "engine.sock");
    const service = await startEngineService({
      socketPath,
      storageRoot: join(root, "state"),
      databasePath: join(root, "rerail.sqlite"),
      embedder: testEmbedder,
      accessToken: "capability",
      createAuditor: () => ({
        openWorkspace: async () => undefined,
        acceptsPath: () => true,
        auditGeneratedEdits: async (_sessionId, batch) => ({ revision: batch.revision, disposition: "continue" as const, violations: [], failures: [] }),
        close: async () => undefined,
      }),
    });
    // What a process that reaches the socket but not the Rerail home would ask the daemon, which runs unsandboxed, to do.
    const elsewhere = join(root, "elsewhere");
    const redirected = {
      storageRoot: join(elsewhere, "sessions"),
      modelPath: join(elsewhere, "model"),
      databasePath: join(elsewhere, "rerail.sqlite"),
      workspaceRoot: root,
    };
    const start = { kind: "session-start", eventId: "start", session: { host: "claude-code", sessionId: "s", cwd: root } } as const;
    try {
      for (const endpoint of [{ socketPath }, { socketPath, accessToken: "not-the-capability" }]) {
        await expect(ingestEngineEvent(endpoint, start, redirected)).rejects.toThrow(/\(401\).*engine-access-required/u);
      }
      expect(existsSync(elsewhere)).toBe(false);
      await expect(ingestEngineEvent({ socketPath, accessToken: "capability" }, start)).resolves.toEqual([]);
    } finally {
      await service.close();
    }
  });

  it("skips a refused socket directory for the next candidate, made for this user alone", async () => {
    const root = await temporaryRoot();
    const squatted = join(root, "tmp", "rerail-squatted");
    await mkdir(squatted, { recursive: true });
    await chmod(squatted, 0o777);
    const fallback = join(root, "home", "run", "engine.sock");
    expect(await chooseEngineSocketPath([join(squatted, "engine.sock"), fallback])).toBe(fallback);
    const info = await lstat(join(root, "home", "run"));
    expect(info.uid).toBe(process.getuid!());
    expect(info.mode & 0o777).toBe(0o700);
  });

  it("skips a socket path too long to bind, and names every refusal with its fix when none is left", async () => {
    const root = await temporaryRoot();
    const tooLong = join(root, "x".repeat(120), "engine.sock");
    const owned = join(root, "owned");
    await mkdir(owned, { mode: 0o700 });
    const fits = join(root, "fits", "engine.sock");
    expect(await chooseEngineSocketPath([tooLong, fits])).toBe(fits);
    expect(existsSync(join(root, "x".repeat(120)))).toBe(false);

    // Seen as another user's directory: only they or an administrator can remove it from a shared temp directory.
    const refusal = chooseEngineSocketPath([join(owned, "engine.sock"), tooLong], process.getuid!() + 1);
    await expect(refusal).rejects.toThrow(new RegExp(`${owned}: it is owned by uid \\d+, not \\d+\\. Another user .*administrator must remove ${owned}`, "u"));
    await expect(refusal).rejects.toThrow(new RegExp(`${tooLong} is \\d+ bytes.*RERAIL_HOME`, "u"));
  });

  /** A socket path under `root` of exactly `bytes` bytes. */
  const pathOf = (root: string, bytes: number): string => join(root, "d".repeat(bytes - Buffer.byteLength(join(root, "d", "engine.sock")) + 1), "engine.sock");

  // `sun_path` holds 108 bytes on Linux and 104 on macOS, each with the name's terminating NUL.
  it.each([["linux", 107], ["darwin", 103]] as const)("takes a socket path of %s's longest, %i bytes, and skips one a byte longer", async (platform, limit) => {
    const root = await temporaryRoot();
    const fits = pathOf(root, limit);
    expect(Buffer.byteLength(fits)).toBe(limit);
    expect(await chooseEngineSocketPath([pathOf(root, limit + 1), fits], process.getuid!(), platform)).toBe(fits);
  });

  // Linux also binds a full 108-byte `sun_path` with no terminating NUL; Rerail stops a byte short, as for macOS.
  it.runIf(process.platform === "linux")("binds a socket at the longest path Rerail takes on Linux, and Linux refuses one past sun_path", async () => {
    const root = await temporaryRoot();
    const listening = async (socketPath: string): Promise<void> => {
      await mkdir(dirname(socketPath), { recursive: true, mode: 0o700 });
      const server = createServer();
      await new Promise<void>((resolve, reject) => server.once("error", reject).listen(socketPath, resolve));
      await new Promise<void>(resolve => server.close(() => resolve()));
    };
    await expect(listening(pathOf(root, 107))).resolves.toBeUndefined();
    await expect(listening(pathOf(join(root, "over"), 109))).rejects.toThrow(/EINVAL|ENAMETOOLONG/u);
  });
});

describe("starting the engine on a pipe name another process holds", () => {
  // Off Windows a `\\.\pipe\` name is a relative path: a Unix socket in the current folder, which refuses a second
  // listener with EADDRINUSE as a held Windows pipe does, so these run on every platform.
  const CAPABILITY = "PERSISTENT-DASHBOARD-CAPABILITY";
  let root: string;
  let previousFolder: string;
  let pipeCount = 0;

  beforeEach(async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-held-pipe-"));
    previousFolder = process.cwd();
    process.chdir(root);
  });

  afterEach(async () => {
    process.chdir(previousFolder);
    await removeFolder(root);
  });

  const pipeName = (): string => String.raw`\\.\pipe\rerail-test-${process.pid}-${Date.now().toString(16)}-${pipeCount++}`;

  it("never shows a holder that is not this home's engine the capability, and listens on the fallback", async () => {
    const pipe = pipeName();
    const seen: string[] = [];
    // A 400 to everything is what this home's engine answers an empty event, so it once stopped every start.
    const squatter = createHttpServer((incoming, response) => {
      const chunks: Buffer[] = [];
      incoming.on("data", (chunk: Buffer) => chunks.push(chunk));
      incoming.on("end", () => {
        seen.push(JSON.stringify({ method: incoming.method, url: incoming.url, headers: incoming.headers, body: Buffer.concat(chunks).toString("utf8") }));
        response.writeHead(400).end("{}");
      });
    });
    await new Promise<void>(resolve => squatter.listen(pipe, resolve));
    try {
      const service = await startEngineService({ socketPath: pipe, storageRoot: join(root, "state"), accessToken: CAPABILITY, pipeFallback: () => `${pipe}-fallback` });
      try {
        expect(service.socketPath).toBe(`${pipe}-fallback`);
        expect(seen).toHaveLength(1);
        expect(seen.join("\n")).not.toContain(CAPABILITY);
      } finally {
        await service.close();
      }
    } finally {
      await new Promise<void>(resolve => squatter.close(() => resolve()));
    }
  });

  it("starts no second engine beside this home's own, and moves aside for another home's", async () => {
    const pipe = pipeName();
    const own = await startEngineService({ socketPath: pipe, storageRoot: join(root, "state"), accessToken: CAPABILITY });
    try {
      await expect(startEngineService({ socketPath: pipe, storageRoot: join(root, "state"), accessToken: CAPABILITY, pipeFallback: () => `${pipe}-1` }))
        .rejects.toThrow(/in use by a running daemon/u);
      const other = await startEngineService({ socketPath: pipe, storageRoot: join(root, "other"), accessToken: "another-home", pipeFallback: () => `${pipe}-2` });
      expect(other.socketPath).toBe(`${pipe}-2`);
      await other.close();
    } finally {
      await own.close();
    }
  });
});

describe("engine proof", () => {
  /** A folder for the engine's socket: under SOCKET_ROOT off Windows, where a Unix socket path must stay short. */
  const proofRoot = async (): Promise<string> => await mkdtemp(join(process.platform === "win32" ? tmpdir() : SOCKET_ROOT, "rerail-engine-proof-"));

  it("is answered only by an engine holding the capability, and the challenge is all a holder sees", async () => {
    const root = await proofRoot();
    const socketPath = process.platform === "win32"
      ? String.raw`\\.\pipe\rerail-proof-${process.pid}-${Date.now().toString(16)}`
      : join(root, "engine.sock");
    const service = await startEngineService({ socketPath, storageRoot: join(root, "state"), accessToken: "capability" });
    try {
      expect(await engineHoldsCapability(socketPath, "capability")).toBe(true);
      expect(await engineHoldsCapability(socketPath, "not-the-capability")).toBe(false);
      const challenge = "ab".repeat(32);
      expect(engineProof("capability", challenge)).not.toContain("capability");
      expect(engineProof("capability", challenge)).not.toBe(engineProof("another", challenge));
    } finally {
      await service.close();
      await removeFolder(root);
    }
  });

  it("refuses a holder whose answer is no proof, a multibyte one of the proof's length among them, without throwing", async () => {
    const root = await proofRoot();
    const socketPath = process.platform === "win32"
      ? String.raw`\\.\pipe\rerail-proof-forged-${process.pid}-${Date.now().toString(16)}`
      : join(root, "engine.sock");
    // 64 characters, as a proof has, but 128 bytes in UTF-8; then a proof in upper case, and a number.
    const answers = [JSON.stringify({ proof: "é".repeat(64) }), JSON.stringify({ proof: "AB".repeat(32) }), JSON.stringify({ proof: 7 })];
    const squatter = createHttpServer((_incoming, response) => {
      response.end(answers.shift() ?? "{}");
    });
    await new Promise<void>(resolve => squatter.listen(socketPath, resolve));
    try {
      expect(await engineHoldsCapability(socketPath, "capability")).toBe(false);
      expect(await engineHoldsCapability(socketPath, "capability")).toBe(false);
      expect(await engineHoldsCapability(socketPath, "capability")).toBe(false);
    } finally {
      await new Promise<void>(resolve => squatter.close(() => resolve()));
      await removeFolder(root);
    }
  });
});

describe("an engine pipe another process holds", () => {
  // Built as the daemon builds them, so a name written by hand cannot drift from what the daemon and its control read.
  const PIPE = enginePipeName(join(tmpdir(), "rerail.sqlite"));
  const FALLBACK = fallbackPipe(PIPE, () => Buffer.from("89abcdef", "hex"));
  const held = Object.assign(new Error("listen EADDRINUSE"), { code: "EADDRINUSE" });

  it("listens on the fallback pipe when the holder is not this home's engine, and says where", async () => {
    const listened: string[] = [];
    const where = await listenBesideHeldPipe(PIPE, held, {
      listen: async path => {
        listened.push(path);
      },
      ownEngineAnswers: async () => false,
      fallback: () => FALLBACK,
    });
    expect(where).toBe(FALLBACK);
    expect(listened).toEqual([FALLBACK]);
    expect(isEnginePipe(pipeNameOf(where))).toBe(true);
  });

  it("starts no second daemon beside this home's own engine, and keeps the error without a fallback", async () => {
    const listen = async (): Promise<void> => {
      throw new Error("must not listen");
    };
    await expect(listenBesideHeldPipe(PIPE, held, { listen, ownEngineAnswers: async () => true, fallback: () => FALLBACK }))
      .rejects.toThrow(/in use by a running daemon/u);
    await expect(listenBesideHeldPipe(PIPE, held, { listen, ownEngineAnswers: async () => false, fallback: undefined })).rejects.toBe(held);
  });

  it("listens on the fallback pipe without asking a holder that capped the name's instances, as no engine does", async () => {
    const capped = Object.assign(new Error("listen EBUSY"), { code: "EBUSY" });
    const listened: string[] = [];
    const asked: string[] = [];
    const where = await listenBesideHeldPipe(PIPE, capped, {
      listen: async path => {
        listened.push(path);
      },
      ownEngineAnswers: async path => {
        asked.push(path);
        return true;
      },
      fallback: () => FALLBACK,
    });
    expect({ where, listened, asked }).toEqual({ where: FALLBACK, listened: [FALLBACK], asked: [] });
    await expect(listenBesideHeldPipe(PIPE, capped, { listen: async () => undefined, ownEngineAnswers: async () => false, fallback: undefined }))
      .rejects.toBe(capped);
  });

  it("keeps a failure that says nothing of a holder as the start's own, and neither asks nor moves", async () => {
    const refused = Object.assign(new Error("listen EPERM"), { code: "EPERM" });
    const done: string[] = [];
    await expect(listenBesideHeldPipe(PIPE, refused, {
      listen: async path => {
        done.push(`listen ${path}`);
      },
      ownEngineAnswers: async path => {
        done.push(`ask ${path}`);
        return false;
      },
      fallback: () => FALLBACK,
    })).rejects.toBe(refused);
    expect(done).toEqual([]);
  });
});

/** Opens the pipe `path` as a client until an open fails, in one turn: the handles it holds, and the code that ended it. */
function takeEveryInstance(path: string): { taken: number[]; ended: string | undefined } {
  const taken: number[] = [];
  for (;;) {
    try {
      taken.push(openSync(path, "r+"));
    } catch (error) {
      return { taken, ended: (error as NodeJS.ErrnoException).code };
    }
  }
}

/** What a listen of `server` on `path` comes to: `listening`, or the code it failed with; `afterBind` runs once it bound. */
async function listenOutcome(server: Server, path: string, afterBind: () => void): Promise<string | undefined> {
  const { promise, resolve } = Promise.withResolvers<string | undefined>();
  server.once("error", (error: NodeJS.ErrnoException) => resolve(error.code));
  server.once("listening", () => resolve("listening"));
  server.listen(path);
  afterBind();
  return await promise;
}

describe.runIf(process.platform === "win32")("starting the engine beside a pipe whose holder allows one instance", () => {
  const CAPABILITY = "PERSISTENT-DASHBOARD-CAPABILITY";
  let root: string;
  let pipeCount = 0;
  const pipeName = (): string => String.raw`\\.\pipe\rerail-test-${process.pid}-${Date.now().toString(16)}-capped-${pipeCount++}`;

  beforeEach(async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-capped-pipe-"));
  });

  afterEach(async () => {
    await removeFolder(root);
  });

  it("listens on the fallback pipe, and leaves the holder's one instance free", async () => {
    const pipe = pipeName();
    const holder = await oneInstanceHolder(pipe);
    try {
      const service = await startEngineService({ socketPath: pipe, storageRoot: join(root, "state"), accessToken: CAPABILITY, pipeFallback: () => `${pipe}-fallback` });
      try {
        expect(service.socketPath).toBe(`${pipe}-fallback`);
        // Nothing connected to the holder, the proof included: its one instance still takes a client.
        closeSync(openSync(pipe, "r+"));
      } finally {
        await service.close();
      }
    } finally {
      await holder.end();
    }
  });

  it("listens on the fallback pipe while a client holds the holder's one instance", async () => {
    const pipe = pipeName();
    const holder = await oneInstanceHolder(pipe);
    const client = openSync(pipe, "r+");
    try {
      const service = await startEngineService({ socketPath: pipe, storageRoot: join(root, "state"), accessToken: CAPABILITY, pipeFallback: () => `${pipe}-fallback` });
      await service.close();
      expect(service.socketPath).toBe(`${pipe}-fallback`);
    } finally {
      closeSync(client);
      await holder.end();
    }
  });

  // An engine whose loop is busy leaves its pipe with no free instance for a while: a start beside it must still ask
  // it for the proof, so a listen there must read as held by a holder that may make more instances.
  it("reads a Node pipe server with every instance taken as EADDRINUSE, never as a capped holder's EBUSY", async () => {
    const pipe = pipeName();
    const engine = createServer();
    await new Promise<void>(resolve => engine.listen(pipe, resolve));
    const second = createServer();
    // Node binds a pipe within `listen`, so the bind meets every instance taken in the same turn.
    const { taken, ended } = takeEveryInstance(pipe);
    const outcome = listenOutcome(second, pipe, () => {
      for (const handle of taken) closeSync(handle);
    });
    try {
      expect({ taken: taken.length > 0, ended, outcome: await outcome }).toEqual({ taken: true, ended: "EBUSY", outcome: "EADDRINUSE" });
    } finally {
      second.close();
      await new Promise<void>(resolve => engine.close(() => resolve()));
    }
  });
});

describe("the answer to a hook's event", () => {
  it("carries the event's deliveries when the facts beside them cannot be told, and what its runtime holds to a hook that asks once they can", async () => {
    const root = await realpath(await mkdtemp(join(process.platform === "win32" ? tmpdir() : SOCKET_ROOT, "rerail-event-answer-")));
    const databasePath = join(root, "rerail.sqlite");
    const repositoryKey = startTestRepository(root, databasePath);
    const socketPath = process.platform === "win32"
      ? String.raw`\\.\pipe\rerail-event-answer-${process.pid}-${Date.now().toString(16)}`
      : join(root, "engine.sock");
    let readable = false;
    const held: (HeldSession | undefined)[] = [];
    const service = await startEngineService({
      socketPath,
      storageRoot: join(root, "state"),
      databasePath,
      embedder: testEmbedder,
      createAuditor: () => ({
        openWorkspace: async () => undefined,
        acceptsPath: () => true,
        auditGeneratedEdits: async (_sessionId, batch) => ({ revision: batch.revision, disposition: "continue" as const, violations: [], failures: [] }),
        close: async () => undefined,
      }),
      answerFacts: (_event, _deliveries, readHeld) => {
        const session = readHeld();
        held.push(session);
        if (!readable) throw new Error("the daemon could not tell its dashboard");
        return { dashboard: null, ...(session?.sessionNote === undefined ? {} : { sessionNote: session.sessionNote }) };
      },
    });
    const start = (sessionId: string) => ({ kind: "session-start", eventId: `${sessionId}:start`, session: { host: "claude-code", sessionId, cwd: root } } as const);
    try {
      // The hook reads what the answer lacks after it, as from a daemon older than these facts.
      await expect(ingestEngineEventAnswer({ socketPath }, start("unread"))).resolves.toEqual({ eventId: "unread:start", deliveries: [] });
      readable = true;
      await expect(ingestEngineEventAnswer({ socketPath }, start("read")))
        .resolves.toEqual({ eventId: "read:start", deliveries: [], dashboard: null, sessionNote: { setUp: false } });
      // What the runtime that took each start holds of its session: its checkout, and a note for a repository not set up.
      const session = { checkout: { repositoryKey, checkoutPath: root }, sessionNote: { setUp: false } };
      expect(held).toEqual([session, session]);
    } finally {
      await service.close();
      await removeFolder(root);
    }
  });
});

/** A listener on a fresh engine socket (a pipe on Windows) in this process, which takes each connection and answers nothing. */
async function silentListener(): Promise<{ socketPath: string; close(): Promise<void> }> {
  const root = process.platform === "win32" ? undefined : await mkdtemp(join(SOCKET_ROOT, "rerail-silent-"));
  const socketPath = root === undefined ? String.raw`\\.\pipe\rerail-silent-${process.pid}-${Date.now()}` : join(root, "engine.sock");
  const connections = new Set<Socket>();
  const server = createServer(socket => connections.add(socket));
  await new Promise<void>(resolve => server.listen(socketPath, resolve));
  return {
    socketPath,
    close: async () => {
      for (const socket of connections) socket.destroy();
      await new Promise<void>(resolve => server.close(() => resolve()));
      if (root !== undefined) await removeFolder(root);
    },
  };
}

describe("an event sent after a proof on the engine", () => {
  const event = { kind: "session-end", eventId: "end", sessionId: "s" } as const;

  /** The wait each request hands its socket while `body` runs, through a stand-in timer that fires once it is handed. */
  const socketWaits = async (body: () => Promise<void>): Promise<number[]> => {
    const waits: number[] = [];
    const timer = vi.spyOn(ClientRequest.prototype, "setTimeout").mockImplementation(function (this: ClientRequest, ms: number, fire?: () => void) {
      waits.push(ms);
      process.nextTick(() => fire?.());
      return this;
    });
    try {
      await body();
    } finally {
      timer.mockRestore();
    }
    return waits;
  };

  it("hands its socket what the proof left of the wait it shares with it, 1 ms once the proof took it all, and its own 30 s with no proof", async () => {
    const silent = await silentListener();
    try {
      const waits = await socketWaits(async () => {
        await expect(ingestEngineEvent({ socketPath: silent.socketPath, proofTookMs: ENGINE_ANSWER_WAIT_MS - 300 }, event)).rejects.toThrow("Kernel request timed out");
        await expect(ingestEngineEvent({ socketPath: silent.socketPath, proofTookMs: ENGINE_ANSWER_WAIT_MS + 1_000 }, event)).rejects.toThrow("Kernel request timed out");
        await expect(ingestEngineEvent({ socketPath: silent.socketPath }, event)).rejects.toThrow("Kernel request timed out");
      });
      // At least 1 ms, since a socket reads 0 as no limit.
      expect(waits).toEqual([300, 1, ENGINE_ANSWER_WAIT_MS]);
    } finally {
      await silent.close();
    }
  });

  it("keeps what the proof left of its wait, from its own request, however long the hook waited for its session's lock in between", async () => {
    const home = await mkdtemp(join(tmpdir(), "rerail-proof-then-lock-"));
    const silent = await silentListener();
    // daemon.json records another pid than any holder Windows names, so the engine on the pipe proves itself first.
    await writeFile(join(home, "daemon.json"), JSON.stringify({ pid: process.pid, host: "127.0.0.1", port: 1, url: "http://127.0.0.1:1/", version: "test",
      databasePath: join(home, "rerail.sqlite"), kernelSocketPath: String.raw`\\.\pipe\rerail-engine-proof-then-lock`, ui: "bundled", startedAt: 1, packageRoot: home, accessToken: "capability" }));
    let now = 1_700_000_000_000;
    const clock = vi.spyOn(Date, "now").mockImplementation(() => now);
    try {
      const proving = fakeMachine("win32", { env: { RERAIL_HOME: home }, capabilityProof: async () => {
        now += 12_000;
        return "proved";
      } });
      const endpoint = await engineEndpointAt(String.raw`\\.\pipe\rerail-engine-proof-then-lock`, undefined, proving);
      // The hook then waits 31 s for its session's lock, held by another hook of the session.
      now += 31_000;
      const waits = await socketWaits(async () => {
        await expect(ingestEngineEvent({ ...endpoint, socketPath: silent.socketPath }, event)).rejects.toThrow("Kernel request timed out");
      });
      expect(waits).toEqual([ENGINE_ANSWER_WAIT_MS - 12_000]);
    } finally {
      clock.mockRestore();
      await silent.close();
      await removeFolder(home);
    }
  });
});

describe("the holder of a Windows named pipe", () => {
  it.runIf(process.platform === "win32")("is the process serving it, as Windows records it, with when that process started, and no one once it stops", async () => {
    const pipe = String.raw`\\.\pipe\rerail-holder-${process.pid}-${Date.now()}`;
    const server = createServer(socket => socket.destroy());
    await new Promise<void>(resolve => server.listen(pipe, resolve));
    try {
      const holder = await pipeServer(pipe, 1_000);
      expect(holder?.pid).toBe(process.pid);
      // Node starts a moment after Windows creates its process.
      const nodeStarted = Date.now() - process.uptime() * 1_000;
      expect(holder?.startedAt).toBeLessThanOrEqual(nodeStarted + 1_000);
      expect(holder?.startedAt).toBeGreaterThan(nodeStarted - 10_000);
    } finally {
      await new Promise<void>(resolve => server.close(() => resolve()));
    }
    expect(await pipeServer(pipe, 0)).toBeUndefined();
  });

  it.runIf(process.platform === "win32")("is not named while another client keeps the pipe's one instance open for the whole wait, says so with no pid, and leaves this process's timers running meanwhile", async () => {
    const pipe = String.raw`\\.\pipe\rerail-holder-busy-${process.pid}-${Date.now()}`;
    // The pipe's one instance is served by another process, which never lists it again once a client takes it.
    const holder = await oneInstanceHolder(pipe);
    const client = connect(pipe).on("error", () => undefined);
    try {
      await once(client, "connect");
      const fired: number[] = [];
      setTimeout(() => fired.push(Date.now()), 50);
      const answers = [await pipeServer(pipe, 0), await pipeServer(pipe, 1_000)];
      const answeredAt = Date.now();
      // An answer with no pid, which tells a pipe kept busy from one no one holds.
      expect(answers.map(answer => answer !== undefined && typeof answer.pid !== "number")).toEqual([true, true]);
      // The timer fired while the lookup waited for a free instance, before its answer.
      expect(fired).toHaveLength(1);
      expect(fired[0]).toBeLessThan(answeredAt);
    } finally {
      client.destroy();
      await holder.end();
    }
  });

  it.runIf(process.platform !== "win32")("is no one off Windows, which has no named pipes", async () => {
    expect(await pipeServer(String.raw`\\.\pipe\rerail-holder-${process.pid}`, 0)).toBeUndefined();
  });
});
