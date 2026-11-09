import { spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import { chmod, lstat, mkdir, mkdtemp, rm, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { createServer } from "node:net";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { chooseEngineSocketPath, ingestEngineEvent, listenBesideHeldPipe, preparePrivateDirectory, startEngineService } from "../../src/runtime/engine-service.js";
import { testEmbedder } from "../support/hook-events.js";
import { startTestRepository } from "../support/started-repository.js";

describe.runIf(process.platform !== "win32")("engine service socket directory", () => {
  const roots: string[] = [];

  async function temporaryRoot(): Promise<string> {
    const root = await mkdtemp(join(tmpdir(), "rerail-socket-"));
    roots.push(root);
    return root;
  }

  afterEach(async () => {
    await Promise.all(roots.splice(0).map(root => rm(root, { recursive: true, force: true })));
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
});

describe("an engine pipe another process holds", () => {
  const PIPE = String.raw`\.\pipe\rerail-0123456789abcdef01234567`;
  const held = Object.assign(new Error("listen EADDRINUSE"), { code: "EADDRINUSE" });

  it("listens on the fallback pipe when the holder is not this home's engine, and says where", async () => {
    const listened: string[] = [];
    const where = await listenBesideHeldPipe(PIPE, held, {
      listen: async path => {
        listened.push(path);
      },
      ownEngineAnswers: async () => false,
      fallback: () => `${PIPE}-89abcdef`,
    });
    expect(where).toBe(`${PIPE}-89abcdef`);
    expect(listened).toEqual([`${PIPE}-89abcdef`]);
  });

  it("starts no second daemon beside this home's own engine, and keeps the error without a fallback", async () => {
    const listen = async (): Promise<void> => {
      throw new Error("must not listen");
    };
    await expect(listenBesideHeldPipe(PIPE, held, { listen, ownEngineAnswers: async () => true, fallback: () => `${PIPE}-1` }))
      .rejects.toThrow(/in use by a running daemon/u);
    await expect(listenBesideHeldPipe(PIPE, held, { listen, ownEngineAnswers: async () => false, fallback: undefined })).rejects.toBe(held);
  });
});
