import { execFile, spawn, type ChildProcess } from "node:child_process";
import { once } from "node:events";
import { existsSync } from "node:fs";
import { mkdir, mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { promisify } from "node:util";
import { describe, expect, test } from "vitest";
import type { DaemonState } from "../src/daemon.js";
import { startTestRepository } from "../test/support/started-repository.js";

describe("Rerail daemon CLI", () => {
  test("starts in dev mode, exposes health, and stops through the CLI", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-daemon-cli-"));
    const stateFile = join(root, "daemon.json");
    const database = join(root, "rerail.sqlite");
    let child: ChildProcess | undefined;
    try {
      child = spawn(process.execPath, [
        join(process.cwd(), "dist", "daemon.js"),
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
      const health = await fetch(new URL("/healthz", state.url));
      expect(health.status).toBe(200);
      expect(health.ok).toBe(true);
      expect(await health.json()).toMatchObject({ pid: child.pid, database: { path: database }, ui: "dev" });
      // `--dev` serves the committed `ui/` source straight from the checkout, not `dist/ui`.
      const page = await fetch(state.url);
      expect(page.status).toBe(200);
      expect(page.headers.get("content-type")).toBe("text/html; charset=utf-8");
      expect(await page.text()).toBe(await readFile(join(process.cwd(), "ui", "index.html"), "utf8"));
      const daemonClosed = once(child, "close");
      const stop = spawn(process.execPath, [join(process.cwd(), "dist/cli.js"), "daemon", "stop"], {
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
      const start = spawn(process.execPath, [join(process.cwd(), "dist", "cli.js"), "daemon", "start", "--host", "0.0.0.0", "--port", "0"], {
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
          [join(process.cwd(), "dist", "cli.js"), "state", "--repo", workspace, ...extra],
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
});
