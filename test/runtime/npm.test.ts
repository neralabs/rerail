import * as childProcess from "node:child_process";
import { EventEmitter, once } from "node:events";
import { chmod, mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { delimiter, dirname, join } from "node:path";
import { PassThrough } from "node:stream";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("node:child_process", { spy: true });

import { cleanCommandError, CommandTimeoutError, environmentForNode, runCommand, runNpm, windowsNpmCliPath } from "../../src/runtime/npm.js";
import { removeFolder } from "../support/platform/remove-folder.js";
import { fromDriveRoot, fullyQualified } from "../support/platform/windows-paths.js";

const temporaryDirectories: string[] = [];
afterEach(async () => {
  vi.unstubAllEnvs();
  await Promise.all(temporaryDirectories.splice(0).map(async directory => await removeFolder(directory)));
});

/** The file name of a program called `name` on this platform: Windows runs only a file with an extension in PATHEXT. */
const program = (name: string): string => (process.platform === "win32" ? `${name}.exe` : name);

async function binDirectory(...files: string[]): Promise<string> {
  const bin = await mkdtemp(join(tmpdir(), "rerail-npm-"));
  temporaryDirectories.push(bin);
  for (const file of files) {
    await writeFile(join(bin, file), "#!/bin/sh\n");
    await chmod(join(bin, file), 0o755);
  }
  return bin;
}

async function fakeNpm(body: string): Promise<string> {
  const bin = await binDirectory();
  await writeFile(join(bin, "npm"), `#!/bin/sh\n${body}\n`);
  await chmod(join(bin, "npm"), 0o755);
  return bin;
}

/** A spawned child as `runCommand` sees one, whose events a test emits itself. */
type FakeChild = EventEmitter & { stdout: PassThrough; stderr: PassThrough; exitCode: number | null; signalCode: NodeJS.Signals | null };

/** Makes the next spawn return a child that `run` drives from the next tick, once the caller is listening, as Node's does. */
function spawnsOnce(run: (child: FakeChild) => void): void {
  vi.mocked(childProcess.spawn).mockImplementationOnce(() => {
    const child: FakeChild = Object.assign(new EventEmitter(), { stdout: new PassThrough(), stderr: new PassThrough(), exitCode: null, signalCode: null });
    process.nextTick(() => run(child));
    return child as unknown as childProcess.ChildProcess;
  });
}

function exit(child: FakeChild, code = 0): void {
  child.exitCode = code;
  child.emit("exit", code, null);
}

/** Ends `child`'s output and emits "close" once it is read to its end, as Node does. */
async function close(child: FakeChild): Promise<void> {
  child.stdout.end();
  child.stderr.end();
  await Promise.all([once(child.stdout, "end"), once(child.stderr, "end")]);
  child.emit("close", child.exitCode, null);
}

function finish(child: FakeChild, code = 0): void {
  exit(child, code);
  void close(child);
}

describe("runNpm on Windows", () => {
  const platform = Object.getOwnPropertyDescriptor(process, "platform")!;
  afterEach(() => {
    Object.defineProperty(process, "platform", platform);
    vi.mocked(childProcess.spawn).mockClear();
  });

  it("puts the requested node first on PATH, so the install scripts npm runs build for it", async () => {
    const node = fullyQualified(join(await binDirectory("node.exe"), "node.exe"));
    spawnsOnce(finish);
    Object.defineProperty(process, "platform", { ...platform, value: "win32" });
    await runNpm(["rebuild", "better-sqlite3"], { node, env: { PATH: "/usr/bin" } });
    const [command, , options] = vi.mocked(childProcess.spawn).mock.calls.at(-1)!;
    expect(command).toBe(node);
    expect((options as childProcess.SpawnOptions).env?.PATH).toBe(`${dirname(node)}${delimiter}/usr/bin`);
  });

  // On Windows only: elsewhere the PATH search finds a Linux or macOS path, which a program start on Windows refuses as a
  // path from the root of the current drive.
  it.runIf(process.platform === "win32")("runs npm under the node.exe PATH finds, never a node.cmd shim before it, which no spawn without a shell starts", async () => {
    const shims = await binDirectory("node.cmd");
    const binaries = await binDirectory("node.exe");
    spawnsOnce(finish);
    Object.defineProperty(process, "platform", { ...platform, value: "win32" });
    await runNpm(["rebuild", "better-sqlite3"], { node: "node", env: { PATH: `${shims};${binaries}`, PATHEXT: ".cmd;.exe" } });
    const [command] = vi.mocked(childProcess.spawn).mock.calls.at(-1)!;
    expect(command).toBe(join(binaries, "node.exe"));
  });

  // On Windows only: elsewhere a path from the root has no drive to change with. A program start read such a name on the
  // drive of the child's folder, and Rerail now starts a program there only by a path from a drive or a server.
  it.runIf(process.platform === "win32")("runs npm under a node named from the root of the current drive, with the current folder's drive", async () => {
    const bin = await binDirectory("node.exe");
    const node = join(bin, "node.exe");
    vi.stubEnv("npm_execpath", "");
    spawnsOnce(finish);
    await inFolder(bin, async () => {
      await runNpm(["rebuild", "better-sqlite3"], { node: fromDriveRoot(node), env: { PATH: String.raw`C:\Windows` } });
      expect(windowsNpmCliPath(fromDriveRoot(node))).toBe(join(bin, "node_modules", "npm", "bin", "npm-cli.js"));
      expect(environmentForNode(fromDriveRoot(node), { PATH: "" })).toEqual({ PATH: bin });
    });
    const [command, args, options] = vi.mocked(childProcess.spawn).mock.calls.at(-1)!;
    expect(command).toBe(node);
    expect(args?.[0]).toBe(join(bin, "node_modules", "npm", "bin", "npm-cli.js"));
    expect((options as childProcess.SpawnOptions).env?.PATH).toBe(`${bin};C:\\Windows`);
  });
});

/** Runs `body` with this process's current folder at `folder`, and puts the folder back after it. */
async function inFolder(folder: string, body: () => Promise<void>): Promise<void> {
  const before = process.cwd();
  process.chdir(folder);
  try {
    await body();
  } finally {
    process.chdir(before);
  }
}

describe("environmentForNode", () => {
  // A Node by a path that names one file on this machine: on Windows one with a drive, since a name from the root of the
  // current drive gets that drive (the case above).
  const otherNode = process.platform === "win32" ? String.raw`C:\opt\other\bin\node` : "/opt/other/bin/node";
  const otherBin = dirname(otherNode);

  it("puts the chosen Node's directory first on PATH so npm's env shebang resolves to it", () => {
    const env = environmentForNode(otherNode, { PATH: "/usr/bin", HOME: "/home/x" });
    expect(env).toEqual({ PATH: `${otherBin}${delimiter}/usr/bin`, HOME: "/home/x" });
  });

  it("keeps the platform's own PATH key and handles an empty PATH", () => {
    expect(environmentForNode(otherNode, { Path: "" })).toEqual({ Path: otherBin });
    expect(environmentForNode(otherNode, {})).toEqual({ PATH: otherBin });
  });

  it("leaves the environment alone for a name PATH cannot resolve, and resolves one it can", async () => {
    const empty = await binDirectory();
    const env = { PATH: empty };
    expect(environmentForNode("node", env)).toBe(env);
    const bin = await binDirectory(program("node"));
    const searched = `${empty}${delimiter}${bin}`;
    expect(environmentForNode("node", { PATH: searched })).toEqual({ PATH: `${bin}${delimiter}${searched}` });
  });

  it("prefixes the running Node's directory by default", () => {
    expect(environmentForNode(process.execPath).PATH?.startsWith(`${dirname(process.execPath)}${delimiter}`)).toBe(true);
  });
});

describe("cleanCommandError", () => {
  it("redacts _authToken and _password values from npm output", () => {
    const text = [
      "//registry.example.test/:_authToken=secret-token",
      "//registry.example.test/npm/:_password=c2VjcmV0",
    ].join("\n");
    const cleaned = cleanCommandError(text);
    expect(cleaned).not.toContain("secret-token");
    expect(cleaned).not.toContain("c2VjcmV0");
    expect(cleaned).toContain(":_authToken=[REDACTED]");
    expect(cleaned).toContain(":_password=[REDACTED]");
  });
});

describe("runCommand", () => {
  it("stops a command that runs past its timeout", async () => {
    const started = Date.now();
    const failure = runCommand(process.execPath, ["-e", "setTimeout(() => {}, 30_000)"], { timeoutMs: 200 });
    await expect(failure).rejects.toThrow(CommandTimeoutError);
    expect(Date.now() - started).toBeLessThan(10_000);
  });

  it("kills a command that ignores SIGTERM once its grace period is over", async () => {
    const started = Date.now();
    const script = "process.on('SIGTERM', () => {}); process.stdout.write('ready'); setInterval(() => {}, 1_000)";
    await expect(runCommand(process.execPath, ["-e", script], { timeoutMs: 500 })).rejects.toThrow(CommandTimeoutError);
    expect(Date.now() - started).toBeLessThan(12_000);
  }, 20_000);

  it("leaves a command that finishes in time alone", async () => {
    await expect(runCommand(process.execPath, ["-e", "process.stdout.write('ok')"], { timeoutMs: 30_000 })).resolves.toEqual({ stdout: "ok", stderr: "" });
  });

  // A process the command started writes once the command has exited (its input, a pipe the command held, ends then),
  // so the output ends only after "exit": the answer is read to its end, as npm's must be when "exit" outruns its last
  // output under load. Not on Windows: there the process the command started dies with it, in the job object libuv puts
  // every child it starts in, and never writes.
  it.skipIf(process.platform === "win32")("reads what is written after the command exits, to the end of its output", async () => {
    const afterExit = "process.stdin.on('end', () => process.stdout.write('late')); process.stdin.resume()";
    const command = `require('node:child_process').spawn(process.execPath, ['-e', ${JSON.stringify(afterExit)}], { stdio: ['pipe', 'inherit', 'inherit'] }); require('node:fs').writeSync(1, 'early '); process.exit(0)`;
    await expect(runCommand(process.execPath, ["-e", command], { timeoutMs: 30_000 })).resolves.toEqual({ stdout: "early late", stderr: "" });
  });

  // A process the command started can hold its output open after the command exits, and "close" then waits for it.
  it("stops waiting on the output of a command that exited once its timeout has run", async () => {
    spawnsOnce((child) => {
      child.stderr.write("npm http fetch GET 200 one\n");
      exit(child);
    });
    await expect(runCommand("npm", ["view", "@neralabs/rerail"], { timeoutMs: 1_000 })).rejects.toThrow(
      "npm did not finish within 1 s and was stopped; its last output:\nnpm http fetch GET 200 one",
    );
  });
});

describe.skipIf(process.platform === "win32")("runNpm", () => {
  it("passes arguments through after quick fetch retries, without adding a registry, userconfig or registry environment", async () => {
    const bin = await fakeNpm('printf "%s\\n" "$@"; printf "registry=%s\\n" "$npm_config_registry"');
    const result = await runNpm(["view", "@neralabs/rerail@latest", "version"], { env: { PATH: bin } });
    expect(result.stdout).toBe("--fetch-retries=1\n--fetch-retry-mintimeout=2000\n--fetch-retry-maxtimeout=10000\nview\n@neralabs/rerail@latest\nversion\nregistry=\n");
  });

  it.each([
    "npm error code E401",
    "npm error 401 Unauthorized - GET https://registry.npmjs.org/@neralabs%2frerail",
  ])("tells the user to refresh credentials when npm reports %s", async (line) => {
    const bin = await fakeNpm(`echo "${line}" >&2; echo "//x/npm/:_password=c2VjcmV0" >&2; exit 1`);
    const failure = runNpm(["view", "@neralabs/rerail"], { env: { PATH: bin } });
    await expect(failure).rejects.toThrow("npm registry credentials are missing or expired. Run `npm login` to refresh them.");
    await expect(failure).rejects.not.toThrow("c2VjcmV0");
  });

  it.each([
    ["ECONNREFUSED", "npm error code ECONNREFUSED\\nnpm error FetchError: request to http://127.0.0.1:9/@neralabs%%2frerail failed, reason: connect ECONNREFUSED 127.0.0.1:9"],
    ["timed out", "npm error code FETCH_ERROR\\nnpm error network timeout at: http://127.0.0.1:9/@neralabs%%2frerail"],
    // Windows ends npm's lines in CRLF; npm 9 and earlier write `npm ERR!`.
    ["ECONNREFUSED", "npm error code ECONNREFUSED\\r\\nnpm error FetchError: request to http://127.0.0.1:9/@neralabs%%2frerail failed, reason: connect ECONNREFUSED 127.0.0.1:9\\r"],
    ["ENOTFOUND", "npm ERR! code ENOTFOUND\\nnpm ERR! request to http://127.0.0.1:9/@neralabs%%2frerail failed, reason: getaddrinfo ENOTFOUND"],
  ])("says which registry npm could not reach (%s) and what to check", async (why, lines) => {
    const bin = await fakeNpm(`printf "${lines}\\n" >&2; exit 1`);
    await expect(runNpm(["view", "@neralabs/rerail"], { env: { PATH: bin } })).rejects.toThrow(
      `could not reach the npm registry (http://127.0.0.1:9, ${why}): check your network, proxy, or registry in ~/.npmrc`,
    );
  });

  it("stops npm once it runs past its timeout, and says which npm command was stopped", async () => {
    const bin = await fakeNpm("exec sleep 30");
    const started = Date.now();
    const failure = runNpm(["install", "--prefix", "/nowhere", "@neralabs/rerail@1.0.0"], { env: { PATH: `${bin}${delimiter}${process.env.PATH ?? ""}` }, timeoutMs: 1_000 });
    await expect(failure).rejects.toThrow(CommandTimeoutError);
    await expect(failure).rejects.toThrow("npm install did not finish within 1 s and was stopped");
    expect(Date.now() - started).toBeLessThan(10_000);
  });

  it("keeps the last lines npm printed before it was stopped, redacted", async () => {
    const bin = await fakeNpm('printf "npm http fetch GET 200 one\\nnpm http fetch GET 200 two\\n//x/:_authToken=secret-token\\n" >&2; exec sleep 30');
    const failure = runNpm(["install", "@neralabs/rerail@1.0.0"], { env: { PATH: `${bin}${delimiter}${process.env.PATH ?? ""}` }, timeoutMs: 1_000 });
    await expect(failure).rejects.toThrow(
      "npm install did not finish within 1 s and was stopped; its last output:\nnpm http fetch GET 200 one\nnpm http fetch GET 200 two\n//x/:_authToken=[REDACTED]",
    );
    await expect(failure).rejects.not.toThrow("secret-token");
    await expect(failure).rejects.toHaveProperty("cause", expect.any(CommandTimeoutError));
  });

  it("leaves other npm failures unchanged", async () => {
    const bin = await fakeNpm('echo "npm error code E404" >&2; exit 1');
    await expect(runNpm(["view", "@neralabs/missing"], { env: { PATH: bin } })).rejects.toThrow(/^npm failed \(1\): npm error code E404$/u);
  });
});
