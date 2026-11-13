import { execFile, spawn, type ChildProcess } from "node:child_process";
import { once } from "node:events";
import { existsSync } from "node:fs";
import { chmod, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { promisify } from "node:util";
import { describe, expect, test } from "vitest";
import { ENTRY_POINTS } from "../src/engine/index.js";
import type { DaemonState } from "../src/runtime/index.js";
import { startTestRepository } from "../test/support/started-repository.js";
import { selfSignedCertificate } from "../test/support/self-signed-certificate.js";

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

  test("refuses a daemon reachable from other machines over plain http unless --insecure-http is passed, before spawning anything", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-daemon-cli-"));
    try {
      for (const remote of [["--host", "0.0.0.0", "--allow-remote"], ["--public-host", "rerail.example"]]) {
        const start = spawn(process.execPath, [join(process.cwd(), "dist", "cli", "cli.js"), "daemon", "start", ...remote, "--port", "0"], {
          env: { ...process.env, RERAIL_HOME: root },
          stdio: ["ignore", "ignore", "pipe"],
        });
        if (start.stderr === null) throw new Error("cli stderr was not piped");
        const chunks: Buffer[] = [];
        start.stderr.on("data", chunk => chunks.push(chunk));
        const [exitCode] = await once(start, "close");
        expect(exitCode).not.toBe(0);
        expect(Buffer.concat(chunks).toString()).toContain("--insecure-http");
        expect(existsSync(join(root, "daemon.json"))).toBe(false);
      }
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  }, 15_000);

  // A TLS key other users can read keeps the daemon from other machines, never from this one: hooks reach it on
  // loopback and keep recording, and the start, the log and the state file say why and how to fix the key.
  test.skipIf(process.platform === "win32")("serves this computer alone, saying why, when other users can read its TLS key", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-daemon-cli-"));
    const cli = join(process.cwd(), "dist", "cli", "cli.js");
    const env = { ...process.env, RERAIL_HOME: root };
    const keyFile = join(root, "key.pem");
    try {
      const { cert, key } = selfSignedCertificate();
      await writeFile(join(root, "cert.pem"), cert);
      await writeFile(keyFile, key);
      await chmod(keyFile, 0o644);
      const remote = ["--host", "0.0.0.0", "--allow-remote", "--tls-cert", join(root, "cert.pem"), "--tls-key", keyFile];
      const { stdout } = await promisify(execFile)(process.execPath, [cli, "daemon", "start", "--dev", "--port", "0", ...remote], { env });
      expect(stdout).toMatch(/running at http:\/\/127\.0\.0\.1:\d+\//u);
      const why = `${keyFile} is open to other users (mode 0644); a TLS private key must be readable by its owner only: chmod 600 "${keyFile}"`;
      expect(stdout).toContain(`Other machines cannot reach this daemon, which serves this computer alone, on loopback, until its TLS key is fixed: ${why}`);
      const state = JSON.parse(await readFile(join(root, "daemon.json"), "utf8")) as Pick<DaemonState, "url" | "accessToken"> & { remoteRefused?: unknown };
      expect(state.remoteRefused).toBe(why);
      const health = await fetch(new URL("/healthz", state.url), { headers: { "x-rerail-access": state.accessToken! } });
      expect(health.status).toBe(200);
      expect(await readFile(join(root, "daemon.log"), "utf8")).toContain(`until its TLS key is fixed: ${why}`);
    } finally {
      await promisify(execFile)(process.execPath, [cli, "daemon", "stop"], { env }).catch(() => undefined);
      await rm(root, { recursive: true, force: true });
    }
  }, 60_000);

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
});
