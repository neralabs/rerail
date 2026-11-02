import { execFile, spawn, type ChildProcess } from "node:child_process";
import { once } from "node:events";
import { existsSync } from "node:fs";
import { mkdir, mkdtemp, readFile, rm } from "node:fs/promises";
import { createServer } from "node:http";
import type { AddressInfo } from "node:net";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { promisify } from "node:util";
import { describe, expect, test } from "vitest";
import { ENTRY_POINTS, openRerailDatabase, steerFeedback } from "../src/engine/index.js";
import type { DaemonState } from "../src/runtime/index.js";
import { startTestRepository } from "../test/support/started-repository.js";

describe("Rerail daemon CLI", () => {
  test("starts in dev mode, exposes health, and stops through the CLI", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-daemon-cli-"));
    const stateFile = join(root, "daemon.json");
    const database = join(root, "rerail.sqlite");
    let child: ChildProcess | undefined;
    try {
      child = spawn(process.execPath, [
        join(process.cwd(), ENTRY_POINTS.daemon),
        "--dev",
        "--port", "0",
        "--state-file", stateFile,
      ], { stdio: ["ignore", "pipe", "ignore"], env: { ...process.env, RERAIL_HOME: root } });
      if (child.stdout === null) throw new Error("daemon stdout was not piped");
      await Promise.race([
        once(child.stdout, "data"),
        once(child, "close").then(([code]) => { throw new Error(`daemon exited with code ${code}`); }),
      ]);
      const state = JSON.parse(await readFile(stateFile, "utf8")) as DaemonState;
      expect(state).toMatchObject({
        pid: child.pid,
        databasePath: database,
        ui: "dev",
      });
      // Every request carries the capability the state file records, as every Rerail client and the browser's cookie do.
      const access = { headers: { "x-rerail-access": state.accessToken! } };
      const health = await fetch(new URL("/healthz", state.url), access);
      expect(health.status).toBe(200);
      expect(health.ok).toBe(true);
      expect(await health.json()).toMatchObject({ pid: child.pid, database: { path: database }, ui: "dev" });
      // `--dev` serves the committed `ui/` source straight from the checkout, not `dist/ui`.
      const page = await fetch(state.url, access);
      expect(page.status).toBe(200);
      expect(page.headers.get("content-type")).toBe("text/html; charset=utf-8");
      expect(await page.text()).toBe(await readFile(join(process.cwd(), "ui", "index.html"), "utf8"));
      const daemonClosed = once(child, "close");
      const stop = spawn(process.execPath, [join(process.cwd(), "dist/cli/cli.js"), "daemon", "stop"], {
        env: { ...process.env, RERAIL_HOME: root },
        stdio: "ignore",
      });
      const [stopExitCode] = await once(stop, "close");
      expect(stopExitCode).toBe(0);
      const [exitCode] = await daemonClosed;
      expect(exitCode).toBe(0);
      expect(existsSync(stateFile)).toBe(false);
    } finally {
      if (child?.exitCode === null) {
        child.kill("SIGKILL");
        await once(child, "close");
      }
      await rm(root, { recursive: true, force: true });
    }
  }, 15_000);

  test("refuses to bind off loopback unless --allow-remote is passed, before spawning anything", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-daemon-cli-"));
    try {
      const start = spawn(process.execPath, [join(process.cwd(), "dist", "cli", "cli.js"), "daemon", "start", "--host", "0.0.0.0", "--port", "0"], {
        env: { ...process.env, RERAIL_HOME: root },
        stdio: ["ignore", "ignore", "pipe"],
      });
      if (start.stderr === null) throw new Error("cli stderr was not piped");
      const chunks: Buffer[] = [];
      start.stderr.on("data", chunk => chunks.push(chunk));
      const [exitCode] = await once(start, "close");
      expect(exitCode).not.toBe(0);
      expect(Buffer.concat(chunks).toString()).toContain("--allow-remote");
      expect(existsSync(join(root, "daemon.json"))).toBe(false);
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  }, 15_000);

  test("reads the start state from the database --database names, not the home's default", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-state-cli-"));
    try {
      const workspace = join(root, "workspace");
      await mkdir(workspace);
      // A configured database the Rerail home does not hold: OMP passes its `databasePath` this way.
      const configured = join(root, "configured", "rules.db");
      await mkdir(dirname(configured));
      startTestRepository(workspace, configured);
      const state = async (...extra: string[]): Promise<string> => {
        const { stdout } = await promisify(execFile)(
          process.execPath,
          [join(process.cwd(), "dist", "cli", "cli.js"), "state", "--repo", workspace, ...extra],
          { env: { ...process.env, RERAIL_HOME: join(root, "home") } },
        );
        return stdout.trim();
      };
      // Started but not set up in the configured database; the home's database knows nothing of it.
      expect(await state("--database", configured)).toBe("not-set-up");
      expect(await state()).toBe("stopped");
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  }, 15_000);

  test("sends an approved feedback note an earlier flush could not deliver once it starts, to where it was approved", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-daemon-feedback-"));
    const received: Array<Record<string, unknown>> = [];
    const arrived = Promise.withResolvers<void>();
    // The stand-in feedback API the note was approved for.
    const api = createServer((request, response) => {
      const chunks: Buffer[] = [];
      request.on("data", chunk => chunks.push(chunk));
      request.on("end", () => {
        received.push(JSON.parse(Buffer.concat(chunks).toString()) as Record<string, unknown>);
        response.writeHead(202).end();
        arrived.resolve();
      });
    });
    api.listen(0, "127.0.0.1");
    await once(api, "listening");
    const destination = `http://127.0.0.1:${(api.address() as AddressInfo).port}/feedback`;
    const databasePath = join(root, "rerail.sqlite");
    const database = openRerailDatabase(databasePath);
    database.insert(steerFeedback).values({
      id: "00000000-0000-4000-8000-000000000001",
      ruleId: "bandit-b101-assert-used",
      packId: "python",
      packVersion: "1.0.0",
      host: "claude-code",
      reason: "asserts are this repo's test style",
      rerailVersion: "9.9.9",
      recordedAt: Date.now(),
      sharing: "queued",
      destination,
    }).run();
    database.$client.close();
    const sharing = (): string | undefined => {
      const reader = openRerailDatabase(databasePath, { readonly: true });
      try {
        return reader.select({ sharing: steerFeedback.sharing }).from(steerFeedback).get()?.sharing;
      } finally {
        reader.$client.close();
      }
    };
    let child: ChildProcess | undefined;
    try {
      // No RERAIL_FEEDBACK_URL here: the note goes to the destination it was approved for.
      const { RERAIL_FEEDBACK_URL: _override, ...env } = process.env;
      child = spawn(process.execPath, [
        join(process.cwd(), ENTRY_POINTS.daemon),
        "--dev",
        "--port", "0",
        "--state-file", join(root, "daemon.json"),
      ], { stdio: ["ignore", "pipe", "ignore"], env: { ...env, RERAIL_HOME: root } });

      await Promise.race([
        arrived.promise,
        once(child, "close").then(([code]) => { throw new Error(`daemon exited with code ${code}`); }),
      ]);
      expect(received).toEqual([expect.objectContaining({ id: "00000000-0000-4000-8000-000000000001", reason: "asserts are this repo's test style" })]);
      // The detached flush settles the note after the API answers; wait for it before the home goes.
      await expect.poll(sharing, { timeout: 5_000 }).toBe("sent");
    } finally {
      if (child?.exitCode === null) {
        child.kill("SIGKILL");
        await once(child, "close");
      }
      api.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 20_000);
});
