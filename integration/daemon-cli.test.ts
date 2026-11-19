import { execFile, spawn, type ChildProcess } from "node:child_process";
import { once } from "node:events";
import { closeSync, existsSync, openSync } from "node:fs";
import { chmod, mkdir, mkdtemp, readdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir, userInfo } from "node:os";
import { basename, dirname, join } from "node:path";
import { pathToFileURL } from "node:url";
import { promisify } from "node:util";
import { describe, expect, test, vi } from "vitest";
import { ENTRY_POINTS, openRerailDatabase, setUpRepository } from "../src/engine/index.js";
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
  // loopback and keep recording, and the start, the log and the state file say why and how to fix the key. A start
  // that asked for other machines fails all the same, so a script or a remote client never takes it for started as
  // asked; one that asked for none succeeds.
  test("serves this computer alone, saying why, when other users can read its TLS key", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-daemon-cli-"));
    const cli = join(process.cwd(), "dist", "cli", "cli.js");
    const env = { ...process.env, RERAIL_HOME: root };
    const keyFile = join(root, "key.pem");
    try {
      const { cert, key } = selfSignedCertificate();
      await writeFile(join(root, "cert.pem"), cert);
      await writeFile(keyFile, key);
      const exposed = await letOthersRead(keyFile, 0o644);
      const tls = ["--tls-cert", join(root, "cert.pem"), "--tls-key", keyFile];
      const started = await rerail(cli, ["daemon", "start", "--dev", "--port", "0", "--host", "0.0.0.0", "--allow-remote", ...tls], env);
      expect(started.code).toBe(1);
      expect(started.stdout).toMatch(/running at http:\/\/127\.0\.0\.1:\d+\//u);
      const state = JSON.parse(await readFile(join(root, "daemon.json"), "utf8")) as Pick<DaemonState, "url" | "accessToken"> & { remoteRefused?: unknown };
      const why = String(state.remoteRefused);
      expect(why).toMatch(new RegExp(`^${exposed}$`, "u"));
      const notice = `This daemon serves this computer alone, over plain http on loopback, until its TLS key is fixed: ${why}`;
      const refusal = `rerail: other machines cannot reach this daemon, though this start asked for them, until its TLS key is fixed: ${why}`;
      expect(started.stdout).toContain(notice);
      expect(started.stderr).toContain(refusal);
      const health = await fetch(new URL("/healthz", state.url), { headers: { "x-rerail-access": state.accessToken! } });
      expect(health.status).toBe(200);
      expect(await readFile(join(root, "daemon.log"), "utf8")).toContain(`until its TLS key is fixed: ${why}`);
      // Found running, it fails again for a start that asks for other machines, and serves one that asks for none.
      const again = await rerail(cli, ["daemon", "start", "--public-host", "rerail.example", ...tls], env);
      expect(again).toMatchObject({ code: 1, stdout: expect.stringContaining("The Rerail daemon is already running.") });
      expect(again.stderr).toContain(refusal);
      expect(await rerail(cli, ["daemon", "start"], env)).toMatchObject({ code: 0, stdout: expect.stringContaining(notice), stderr: expect.not.stringContaining("other machines cannot reach") });
      expect((await rerail(cli, ["status"], env)).stdout).toContain(notice);
      // A TLS start on loopback asks for no other machine: with the same key it serves this computer alone and succeeds.
      expect((await rerail(cli, ["daemon", "stop"], env)).code).toBe(0);
      expect(await rerail(cli, ["daemon", "start", "--dev", "--port", "0", ...tls], env)).toMatchObject({ code: 0, stdout: expect.stringContaining(notice), stderr: expect.not.stringContaining("other machines cannot reach") });
    } finally {
      await rerail(cli, ["daemon", "stop"], env);
      await rm(root, { recursive: true, force: true });
    }
  }, 90_000);

  // A restart at the heap ceiling launches the next daemon with this process's arguments (`process.argv`). A daemon that
  // serves this computer alone comes back as that daemon, saying why in its state file and in daemon.log, never with the
  // arguments that asked for other machines, which would reach them once the key is fixed though no one started it so.
  test("comes back from a heap-ceiling restart on loopback, still saying why, once its TLS key is fixed", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-daemon-cli-"));
    const cli = join(process.cwd(), "dist", "cli", "cli.js");
    const env = { ...process.env, RERAIL_HOME: root };
    const keyFile = join(root, "key.pem");
    const stateFile = join(root, "daemon.json");
    const relaunchFile = join(root, "relaunch.json");
    const log = join(root, "daemon.log");
    const reasonLines = async (why: string): Promise<number> =>
      (await readFile(log, "utf8")).split(`rerail daemon: This daemon serves this computer alone, over plain http on loopback, until its TLS key is fixed: ${why}`).length - 1;
    try {
      const { cert, key } = selfSignedCertificate();
      await writeFile(join(root, "cert.pem"), cert);
      await writeFile(keyFile, key);
      await letOthersRead(keyFile, 0o644);
      // The daemon process as its entry point runs it, then the arguments a restart would relaunch it with.
      const script = join(root, "daemon.mjs");
      await writeFile(script, [
        `const { serveRerailDaemon } = await import(${JSON.stringify(pathToFileURL(join(process.cwd(), "dist", "daemon", "daemon.js")).href)});`,
        "await serveRerailDaemon(process.argv.slice(2));",
        `(await import("node:fs")).writeFileSync(${JSON.stringify(relaunchFile)}, JSON.stringify(process.argv.slice(2)));`,
      ].join("\n"));
      const local = ["--dev", "--port", "0", "--state-file", stateFile];
      const remote = ["--host", "0.0.0.0", "--allow-remote", "--tls-cert", join(root, "cert.pem"), "--tls-key", keyFile];
      launchLogged([script, ...local, ...remote], env, log);
      const relaunch = await vi.waitFor(async () => JSON.parse(await readFile(relaunchFile, "utf8")) as string[], { timeout: 30_000, interval: 100 });
      const why = String((JSON.parse(await readFile(stateFile, "utf8")) as { remoteRefused?: unknown }).remoteRefused);
      expect(relaunch).toEqual([...local, "--remote-refused", why]);
      expect(await reasonLines(why)).toBe(1);
      expect((await rerail(cli, ["daemon", "stop"], env)).code).toBe(0);
      // Launched with those arguments once the key is private, the next daemon still serves this computer alone, and
      // daemon.log says why once more, for this start.
      await makePrivate(keyFile);
      launchLogged([join(process.cwd(), ENTRY_POINTS.daemon), ...relaunch], env, log);
      const state = await vi.waitFor(
        async () => JSON.parse(await readFile(stateFile, "utf8")) as Pick<DaemonState, "url" | "accessToken"> & { remoteRefused?: unknown },
        { timeout: 30_000, interval: 100 },
      );
      expect(state).toMatchObject({ url: expect.stringMatching(/^http:\/\/127\.0\.0\.1:\d+\/$/u), remoteRefused: why });
      expect(await reasonLines(why)).toBe(2);
      const health = await fetch(new URL("/healthz", state.url), { headers: { "x-rerail-access": state.accessToken! } });
      expect(health.status).toBe(200);
    } finally {
      await rerail(cli, ["daemon", "stop"], env);
      await rm(root, { recursive: true, force: true });
    }
  }, 90_000);

  // A --tls-key that names no file this account can read is a mistake in the command, not a key others can read: the
  // start fails with the reason main gives, naming the key, and no daemon runs, on this computer or for others.
  test("refuses to start when --tls-key names no file", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-daemon-cli-"));
    const cli = join(process.cwd(), "dist", "cli", "cli.js");
    const env = { ...process.env, RERAIL_HOME: root };
    const certFile = join(root, "cert.pem");
    const unread = (keyFile: string): [string, string] => [keyFile, `Could not read who may read ${literal(keyFile)}: `];
    try {
      await writeFile(certFile, selfSignedCertificate().cert);
      const folder = join(root, "keys");
      await mkdir(folder);
      const lockedOut = join(root, "key.pem");
      await writeFile(lockedOut, selfSignedCertificate().key);
      // A misspelled name and a path through a file; then paths others can read where this account reads no key: a
      // folder, the certificate (as when --tls-cert and --tls-key are swapped), and a key this account may not read.
      const keys: [string, string][] = [
        unread(join(root, "keyy.pem")), unread(join(certFile, "key.pem")),
        [folder, await letOthersRead(folder, 0o755)], [certFile, await letOthersRead(certFile, 0o644)],
      ];
      if (process.platform === "win32") keys.push(unread(await keyGetAclCannotRead(root)));
      const lockedOutRefusal = await lockOut(lockedOut);
      // Root, and a Windows account whose backup privilege is on (as over ssh, Node opening files for backup), still
      // read that key: for them it is a key this account reads, which serves this computer alone.
      if (!(await readFile(lockedOut).then(() => true, () => false))) keys.push([lockedOut, lockedOutRefusal]);
      for (const [keyFile, refusal] of keys) {
        const remote = ["--host", "0.0.0.0", "--allow-remote", "--tls-cert", certFile, "--tls-key", keyFile];
        const start = promisify(execFile)(process.execPath, [cli, "daemon", "start", "--dev", "--port", "0", ...remote], { env });
        await expect(start).rejects.toMatchObject({ code: 1, stderr: expect.stringMatching(new RegExp(refusal, "u")) });
        expect(existsSync(join(root, "daemon.json"))).toBe(false);
      }
    } finally {
      await promisify(execFile)(process.execPath, [cli, "daemon", "stop"], { env }).catch(() => undefined);
      await rm(root, { recursive: true, force: true });
    }
  }, 60_000);

  // A --tls-cert the https start would refuse is a mistake in the command too. Beside a key others can read, the
  // loopback daemon would serve no certificate, yet the start fails with the message that certificate gets beside a
  // private key, as on main, so the mistake shows now and not once the key is fixed; no daemon runs.
  test("refuses to start when --tls-cert names no certificate for a key others can read", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-daemon-cli-"));
    const cli = join(process.cwd(), "dist", "cli", "cli.js");
    const env = { ...process.env, RERAIL_HOME: root };
    const keyFile = join(root, "key.pem");
    const misspelled = join(root, "certt.pem");
    try {
      await writeFile(keyFile, selfSignedCertificate().key);
      await writeFile(join(root, "other.pem"), selfSignedCertificate().cert);
      await letOthersRead(keyFile, 0o644);
      const notFound = `ENOENT: no such file or directory, open '${literal(misspelled)}'`;
      // A misspelled certificate, on loopback and for other machines; a folder, named though its read error names no
      // file; the key given as the certificate too; another key's certificate.
      const starts: [string[], string][] = [
        [["--tls-cert", misspelled], notFound], [["--host", "0.0.0.0", "--allow-remote", "--tls-cert", misspelled], notFound],
        [["--tls-cert", root], `EISDIR: illegal operation on a directory, read '${literal(root)}'`],
        [["--tls-cert", keyFile], `The TLS certificate ${literal(keyFile)} could not be read as a PEM X\\.509 certificate: `],
        [["--tls-cert", join(root, "other.pem")], "error:\\w+:x509 certificate routines::key values mismatch"],
      ];
      for (const [options, message] of starts) {
        const start = promisify(execFile)(process.execPath, [cli, "daemon", "start", "--dev", "--port", "0", ...options, "--tls-key", keyFile], { env });
        await expect(start).rejects.toMatchObject({ code: 1, stderr: expect.stringMatching(new RegExp(`rerail daemon: ${message}`, "u")) });
        expect(existsSync(join(root, "daemon.json"))).toBe(false);
      }
    } finally {
      await promisify(execFile)(process.execPath, [cli, "daemon", "stop"], { env }).catch(() => undefined);
      await rm(root, { recursive: true, force: true });
    }
  }, 60_000);

  // An empty key used to start a daemon that said it listened on https and failed every handshake: `rerail daemon start`
  // waited its 15 s, said only that the daemon was not ready, and left it running, its engine socket open, so hooks sent
  // there recorded their events. A key that is empty or holds no PEM is served on as one others can read is: this
  // computer alone, over http, with its engine socket, the start naming the file in one line. Such a certificate,
  // refused at main too, still stops the start, naming the file, and no daemon runs.
  test("serves this computer alone and names the file when --tls-key is empty or holds no PEM, hooks on its engine socket recording, and refuses such a --tls-cert", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-daemon-cli-"));
    const home = join(root, "home");
    const repo = join(root, "repo");
    const cli = join(process.cwd(), "dist", "cli", "cli.js");
    const env = { ...process.env, RERAIL_HOME: home };
    const certFile = join(root, "cert.pem");
    const keyFile = join(root, "key.pem");
    const emptyKey = join(root, "empty-key.pem");
    const textKey = join(root, "text-key.pem");
    const emptyCert = join(root, "empty-cert.pem");
    try {
      await startedRepository(repo, home);
      const { cert, key } = selfSignedCertificate();
      const texts: [string, string][] = [[certFile, cert], [keyFile, key], [emptyKey, ""], [textKey, "not a key\n"], [emptyCert, ""]];
      for (const [file, text] of texts) await writeFile(file, text);
      for (const file of [keyFile, emptyKey, textKey]) await makePrivate(file);
      const keys: [string, string][] = [
        [emptyKey, `The TLS private key ${emptyKey} is empty: it must hold a PEM private key`],
        [textKey, `The TLS private key ${textKey} could not be read as a PEM private key: `],
      ];
      for (const [tlsKey, why] of keys) {
        const started = await rerail(cli, ["daemon", "start", "--dev", "--port", "0", "--tls-cert", certFile, "--tls-key", tlsKey], env);
        expect(started).toMatchObject({ code: 0, stdout: expect.stringContaining(`This daemon serves this computer alone, over plain http on loopback, until its TLS key is fixed: ${why}`) });
        const state = JSON.parse(await readFile(join(home, "daemon.json"), "utf8")) as Pick<DaemonState, "url" | "accessToken" | "kernelSocketPath">;
        expect(state.url).toMatch(/^http:\/\/127\.0\.0\.1:\d+\/$/u);
        expect((await fetch(new URL("/healthz", state.url), { headers: { "x-rerail-access": state.accessToken! } })).status).toBe(200);
        expect(await hookRecords(repo, { ...env, RERAIL_ENGINE_SOCKET: state.kernelSocketPath, RERAIL_AUTOSTART: "0" })).toBeGreaterThan(0);
        expect((await rerail(cli, ["daemon", "stop"], env)).code).toBe(0);
      }
      const start = promisify(execFile)(process.execPath, [cli, "daemon", "start", "--dev", "--port", "0", "--tls-cert", emptyCert, "--tls-key", keyFile], { env });
      const refusal = `rerail daemon: The TLS certificate ${literal(emptyCert)} is empty: it must hold a PEM X\\.509 certificate`;
      await expect(start).rejects.toMatchObject({ code: 1, stderr: expect.stringMatching(new RegExp(refusal, "u")) });
      expect(existsSync(join(home, "daemon.json"))).toBe(false);
    } finally {
      await rerail(cli, ["daemon", "stop"], env);
      await rm(root, { recursive: true, force: true });
    }
  }, 120_000);

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

  // A daemon killed without its cleanup leaves daemon.json and the home lock's holder record naming its pid, which the
  // OS can give to any process started since. On Windows a lookup asks about that pid only while the recorded engine
  // pipe is there, so the test keeps it open; there `rerail status` has used its 2 s on the pipe before it could read
  // when the process started, so it keeps the record, and the stop, which has its own wait, removes it. macOS reads no
  // start time, so there the stop ends nothing and says it could not prove whose process it is.
  test.runIf(process.platform !== "darwin")("ends no process a dead daemon's records name, and says the record named no daemon of the home", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-not-daemon-cli-"));
    const stateFile = join(root, "daemon.json");
    const env = { ...process.env, RERAIL_HOME: root };
    const others: ChildProcess[] = [];
    try {
      // A home that is set up: its database is there, so rerail status reports on the daemon.
      const workspace = join(root, "workspace");
      await mkdir(workspace);
      startTestRepository(workspace, join(root, "rerail.sqlite"));
      const daemon = spawn(process.execPath, [join(process.cwd(), ENTRY_POINTS.daemon), "--dev", "--port", "0", "--state-file", stateFile], { stdio: ["ignore", "pipe", "ignore"], env });
      others.push(daemon);
      await Promise.race([once(daemon.stdout!, "data"), once(daemon, "close").then(([code]) => { throw new Error(`daemon exited with code ${code}`); })]);
      const record = await readFile(stateFile, "utf8");
      const state = JSON.parse(record) as DaemonState;
      const closed = once(daemon, "close");
      daemon.kill("SIGKILL");
      await closed;
      if (process.platform === "win32") {
        const pipe = spawn(process.execPath, ["-e", `require("node:net").createServer(socket => socket.pause())
          .listen(process.argv[1], () => process.stdout.write("listening\\n"));`, state.kernelSocketPath], { stdio: ["ignore", "pipe", "ignore"] });
        others.push(pipe);
        await once(pipe.stdout!, "data");
      }
      const unrelated = spawn(process.execPath, ["-e", "setInterval(() => {}, 1e6)"], { stdio: "ignore" });
      others.push(unrelated);
      // The records as the daemon wrote them an hour before the process that has its pid now started.
      const recordsName = async (pid: number): Promise<void> => {
        await writeFile(stateFile, JSON.stringify({ ...state, pid, startedAt: state.startedAt - 3_600_000 }));
        await writeFile(join(root, "locks", "daemon.json"), JSON.stringify({ pid }));
      };
      const cli = async (...args: string[]): Promise<{ code: number; stdout: string }> => await promisify(execFile)(process.execPath, [join(process.cwd(), "dist", "cli", "cli.js"), ...args], { env })
        .then(({ stdout }) => ({ code: 0, stdout }), (error: { code: number; stdout: string }) => ({ code: error.code, stdout: error.stdout }));
      const note = "The process daemon.json named is not a Rerail daemon of this home, so Rerail ended nothing and removed that record.";

      await recordsName(unrelated.pid!);
      const status = await cli("status");
      if (process.platform === "win32") {
        expect(status.code).toBe(4);
        expect(existsSync(stateFile)).toBe(true);
      } else {
        expect(status.code).toBe(3);
        expect(status.stdout).toContain(`${note}\nThe Rerail daemon is not running`);
        expect(existsSync(stateFile)).toBe(false);
      }
      await recordsName(unrelated.pid!);
      expect(await cli("daemon", "stop")).toEqual({ code: 0, stdout: `${note}\nThe Rerail daemon is not running.\n` });
      expect(existsSync(stateFile)).toBe(false);
      expect(unrelated.exitCode === null && unrelated.signalCode === null).toBe(true);
    } finally {
      for (const child of others) {
        if (child.exitCode === null && child.signalCode === null) {
          const closed = once(child, "close");
          child.kill("SIGKILL");
          await closed;
        }
      }
      await rm(root, { recursive: true, force: true });
    }
  }, 30_000);
});

/** Runs the built CLI with `args`, resolving with its exit code and output whatever the code. */
async function rerail(cli: string, args: readonly string[], env: NodeJS.ProcessEnv): Promise<{ code: number | null; stdout: string; stderr: string }> {
  const { promise, resolve } = Promise.withResolvers<{ code: number | null; stdout: string; stderr: string }>();
  const child = execFile(process.execPath, [cli, ...args], { env }, (_error, stdout, stderr) => resolve({ code: child.exitCode, stdout, stderr }));
  return await promise;
}

/** Starts Node with `args`, its stdout and stderr appended to `logFile`, as a daemon's go to daemon.log. */
function launchLogged(args: readonly string[], env: NodeJS.ProcessEnv, logFile: string): void {
  const log = openSync(logFile, "a");
  try {
    spawn(process.execPath, [...args], { env, stdio: ["ignore", log, log] });
  } finally {
    closeSync(log);
  }
}

/** A Git checkout at `repo`, started in the Rerail home `home` with no rule packs, so a hook there records its events. */
async function startedRepository(repo: string, home: string): Promise<void> {
  await mkdir(repo, { recursive: true });
  await mkdir(home, { recursive: true });
  await promisify(execFile)("git", ["init", "-q"], { cwd: repo });
  await writeFile(join(repo, "a.txt"), "a\n");
  const databasePath = join(home, "rerail.sqlite");
  const repositoryKey = startTestRepository(repo, databasePath);
  const database = openRerailDatabase(databasePath);
  try {
    setUpRepository(database, repositoryKey, { packs: [] });
  } finally {
    database.$client.close();
  }
}

/**
 * Runs the built Claude Code hook once in `repo`, on a read, under `env`, its profile a folder of its own, and returns
 * how many lines the checkout's session ledgers hold after it.
 */
async function hookRecords(repo: string, env: NodeJS.ProcessEnv): Promise<number> {
  const profile = join(repo, "..", "profile");
  await mkdir(profile, { recursive: true });
  const input = JSON.stringify({
    session_id: "empty-key-session", cwd: repo, hook_event_name: "PreToolUse", tool_name: "Read", tool_use_id: "toolu_empty_key", tool_input: { file_path: join(repo, "a.txt") },
  });
  const hook = execFile(process.execPath, [join(process.cwd(), ENTRY_POINTS.hookLauncher), process.execPath, join(process.cwd(), ENTRY_POINTS.claudeHook)], {
    cwd: repo, env: { ...env, CLAUDE_PROJECT_DIR: repo, HOME: profile, USERPROFILE: profile },
  });
  hook.stdin?.end(input);
  await once(hook, "close");
  const sessions = join(repo, ".rerail", "sessions");
  if (!existsSync(sessions)) return 0;
  const ledgers = (await readdir(sessions, { recursive: true })).filter(entry => basename(entry) === "events.jsonl");
  const texts = await Promise.all(ledgers.map(async ledger => await readFile(join(sessions, ledger), "utf8")));
  return texts.join("").split("\n").filter(line => line !== "").length;
}

/** `text` as a regular expression source that matches it alone. */
function literal(text: string): string {
  return text.replace(/[\\^$.*+?()[\]{}|]/gu, "\\$&");
}

/**
 * Lets other accounts read `keyFile`: `mode`, or on Windows, which has no mode bits, read granted to BUILTIN\Users, as
 * a key under C:\certs inherits. Returns, as a regular expression source, the whole of the daemon's refusal of it, the
 * command that fixes it included.
 */
async function letOthersRead(keyFile: string, mode: number): Promise<string> {
  const path = literal(keyFile);
  const owner = "; a TLS private key must be readable by its owner only: ";
  if (process.platform !== "win32") {
    await chmod(keyFile, mode);
    return `${path} is open to other users \\(mode ${mode.toString(8).padStart(4, "0")}\\)${owner}chmod 600 "${path}"`;
  }
  await promisify(execFile)("icacls", [keyFile, "/grant", "*S-1-5-32-545:R"], { windowsHide: true });
  // Its folder's ACL may let other accounts read it as well, as Authenticated Users may outside the profile.
  const fix = `icacls "${path}" /inheritance:r /grant:r "${literal(userInfo().username)}:R"`;
  return `${path} can be read by other accounts \\((?:[^)]*, )?BU(?:, [^)]*)?\\)${owner}${fix}`;
}

/**
 * Lets other accounts read `keyFile` and denies this account reading it: mode 0044, or on Windows read granted to
 * BUILTIN\Users and reading its data denied to this account, whose ACL it can still read. Returns the daemon's refusal
 * of it as `letOthersRead` does.
 */
async function lockOut(keyFile: string): Promise<string> {
  const refusal = await letOthersRead(keyFile, 0o044);
  if (process.platform === "win32") await promisify(execFile)("icacls", [keyFile, "/deny", `${userInfo().username}:(RD)`], { windowsHide: true });
  return refusal;
}

/** Makes `keyFile` its owner's alone, as the daemon's printed fix does: mode 0600, or on Windows read for this account alone. */
async function makePrivate(keyFile: string): Promise<void> {
  if (process.platform !== "win32") return await chmod(keyFile, 0o600);
  await promisify(execFile)("icacls", [keyFile, "/inheritance:r", "/grant:r", `${userInfo().username}:R`], { windowsHide: true });
}

/**
 * Windows: a private key this account reads and other accounts may read, whose ACL `Get-Acl` cannot read. It sits in an
 * NTFS alternate data stream (`key.pem:tls`), which Node stats as a file and reads, and whose path `Get-Acl` refuses
 * ("The given path's format is not supported"). Returns its path.
 */
async function keyGetAclCannotRead(folder: string): Promise<string> {
  const file = join(folder, "stream.pem");
  await writeFile(`${file}:tls`, selfSignedCertificate().key);
  await letOthersRead(file, 0o644);
  return `${file}:tls`;
}
