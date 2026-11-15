import { execFile, spawn, type ChildProcess } from "node:child_process";
import { once } from "node:events";
import { chmod, mkdir, mkdtemp, readFile, rm, truncate, writeFile } from "node:fs/promises";
import { createServer as createHttpsServer } from "node:https";
import type { AddressInfo } from "node:net";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { PassThrough, Readable } from "node:stream";
import { promisify } from "node:util";
import { afterEach, describe, expect, expectTypeOf, it } from "vitest";
import { runRerailMcpBridge } from "../../src/api/index.js";
import {
  DAEMON_TRANSPORT_FLAGS,
  DAEMON_TRANSPORT_OPTIONS,
  loadDaemonTls,
  loopbackArguments,
  parseArguments,
  parseDaemonTransport,
  RerailDaemon,
  serveRerailDaemon,
  TlsKeyExposedError,
} from "../../src/daemon/daemon.js";
import { DAEMON_STATE_FILE, daemonFetch, probeDaemon, readDaemonState, type DaemonState } from "../../src/runtime/index.js";
import { selfSignedCertificate } from "../support/self-signed-certificate.js";
import { removeFolder } from "../support/platform/remove-folder.js";

const transport = (argv: readonly string[]) => parseArguments(
  argv,
  { flags: DAEMON_TRANSPORT_FLAGS, options: DAEMON_TRANSPORT_OPTIONS },
  argument => `unknown ${argument}`,
);

describe("daemon transport arguments", () => {
  it("refuses plain http for a daemon reachable from other machines unless --insecure-http is passed", () => {
    expect(() => parseDaemonTransport(transport([]), "0.0.0.0", [])).toThrow(/--insecure-http/u);
    expect(() => parseDaemonTransport(transport([]), "127.0.0.1", ["rerail.example"])).toThrow(/--insecure-http/u);
    expect(parseDaemonTransport(transport(["--insecure-http"]), "0.0.0.0", ["rerail.example"])).toEqual({ insecureHttp: true });
    expect(parseDaemonTransport(transport([]), "127.0.0.1", [])).toEqual({});
    expect(parseDaemonTransport(transport(["--tls-cert", "cert.pem", "--tls-key", "key.pem"]), "0.0.0.0", [])).toEqual({
      tls: { certFile: resolve("cert.pem"), keyFile: resolve("key.pem") },
    });
  });

  it("takes a certificate only with its key, and never together with --insecure-http", () => {
    expect(() => parseDaemonTransport(transport(["--tls-cert", "cert.pem"]), "127.0.0.1", [])).toThrow(/--tls-key/u);
    expect(() => parseDaemonTransport(transport(["--tls-key", "key.pem"]), "127.0.0.1", [])).toThrow(/--tls-cert/u);
    expect(() => parseDaemonTransport(transport(["--tls-cert", "c", "--tls-key", "k", "--insecure-http"]), "0.0.0.0", [])).toThrow();
  });

  it("tells a Windows user to lock the new key down with icacls, which Windows has, and never chmod", () => {
    const refusal = (platform: NodeJS.Platform): string => {
      try {
        parseDaemonTransport(transport([]), "0.0.0.0", [], platform);
      } catch (error) {
        return (error as Error).message;
      }
      throw new Error("expected a refusal");
    };
    expect(refusal("win32")).not.toContain("chmod");
    expect(refusal("win32")).toContain("icacls key.pem /inheritance:r /grant:r");
    expect(refusal("linux")).toContain("chmod 600 key.pem");
  });

  it("leaves out only what reaches other machines when the daemon serves this computer alone", () => {
    const remote = [
      "--state-file", "daemon.json", "--host", "0.0.0.0", "--allow-remote", "--public-host", "rerail.example",
      "--tls-cert", "cert.pem", "--tls-key", "key.pem", "--port", "4737", "--dev", "--workspace-root", "repo",
    ];
    expect(loopbackArguments(remote)).toEqual(["--state-file", "daemon.json", "--port", "4737", "--dev", "--workspace-root", "repo"]);
    expect(loopbackArguments(["--insecure-http", "--host", "::"])).toEqual([]);
    // Every transport flag and option the daemon reads, so one added later cannot ride into the loopback restart.
    const transport = [...DAEMON_TRANSPORT_FLAGS, ...DAEMON_TRANSPORT_OPTIONS.flatMap(option => [option, "value"])];
    expect(loopbackArguments(["--dev", ...transport, "--port", "4737"])).toEqual(["--dev", "--port", "4737"]);
  });

  it("says why it serves this computer alone only from its own loopback restart, never from what a caller passes", () => {
    // A caller that could set it would record a daemon other machines reach as one they cannot.
    expectTypeOf<NonNullable<Parameters<typeof serveRerailDaemon>[1]>>().not.toHaveProperty("remoteRefused");
  });
});

describe("TLS daemon", () => {
  let root: string;
  const daemons: RerailDaemon[] = [];
  const fileHolders: ChildProcess[] = [];
  const originalRerailHome = process.env.RERAIL_HOME;

  afterEach(async () => {
    for (const holder of fileHolders.splice(0)) holder.kill();
    await Promise.all(daemons.splice(0).map(daemon => daemon.close()));
    if (originalRerailHome === undefined) delete process.env.RERAIL_HOME;
    else process.env.RERAIL_HOME = originalRerailHome;
    await removeFolder(root);
  });

  /** An https daemon recording its state in `<root>/home`, as `rerail daemon start --tls-cert … --tls-key …` does. */
  async function startHttps(tls: { cert: string; key: string }): Promise<{ home: string; state: DaemonState }> {
    const home = join(root, "home");
    await mkdir(home, { recursive: true });
    process.env.RERAIL_HOME = home;
    daemons.push(await RerailDaemon.start({
      version: "0.0.0-test",
      databasePath: join(root, "rerail.sqlite"),
      workspaceRoot: root,
      port: 0,
      nativeHealth: () => ({ ok: true, kernelVersion: "1" }),
      stateFile: join(home, DAEMON_STATE_FILE),
      tls,
    }));
    const state = await readDaemonState(join(home, DAEMON_STATE_FILE));
    if (state === undefined) throw new Error("the daemon wrote no state file");
    return { home, state };
  }

  /**
   * A python3 process holding a write lease on `file`, as Samba's kernel oplocks and an NFS server's delegations hold
   * one: another process's open of the file waits for the holder to give it up, which this one never does (it ignores
   * the signal that asks), so the kernel ends the lease only after `/proc/sys/fs/lease-break-time`, 45 s by default.
   * Resolves with "held", or with why the lease could not be taken.
   */
  async function holdLease(file: string): Promise<string> {
    // 1024 is F_SETLEASE on Linux, which older Pythons do not name.
    const script = "import fcntl, os, signal, sys, time\nsignal.signal(signal.SIGIO, signal.SIG_IGN)\n"
      + "fcntl.fcntl(os.open(sys.argv[1], os.O_RDONLY), 1024, fcntl.F_WRLCK)\nprint('held', flush=True)\ntime.sleep(60)\n";
    return await holdWith(spawn("python3", ["-c", script, file], { stdio: ["ignore", "pipe", "pipe"] }));
  }

  /**
   * A PowerShell process holding an oplock on `file` that caches reads, writes and the handle, as Windows' SMB server
   * holds one for a client that caches the file: another process's open that reads the file waits for the holder to
   * acknowledge the break, which this one never does, until it ends. Resolves with "held", or with why it could not.
   */
  async function holdOplock(file: string): Promise<string> {
    // FSCTL_REQUEST_OPLOCK (0x90240) on a handle opened for overlapped reads; the request stays pending while it holds.
    const script = String.raw`$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition @'
using System; using System.Runtime.InteropServices; using Microsoft.Win32.SafeHandles;
public static class Oplock {
  [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
  static extern SafeFileHandle CreateFileW(string name, uint access, uint share, IntPtr security, uint disposition, uint flags, IntPtr template);
  [DllImport("kernel32.dll", SetLastError = true)]
  static extern bool DeviceIoControl(SafeFileHandle file, uint code, IntPtr input, uint inputSize, IntPtr output, uint outputSize, IntPtr returned, IntPtr overlapped);
  [DllImport("kernel32.dll", SetLastError = true)]
  static extern IntPtr CreateEventW(IntPtr security, bool manualReset, bool initialState, IntPtr name);
  public static SafeFileHandle Held;
  public static string Hold(string path) {
    Held = CreateFileW(path, 0x80000000, 7, IntPtr.Zero, 3, 0x40000000, IntPtr.Zero);
    if (Held.IsInvalid) return "open failed: " + Marshal.GetLastWin32Error();
    IntPtr input = Marshal.AllocHGlobal(12), output = Marshal.AllocHGlobal(24), overlapped = Marshal.AllocHGlobal(32);
    for (int at = 0; at < 32; at += 4) { if (at < 24) Marshal.WriteInt32(output, at, 0); Marshal.WriteInt32(overlapped, at, 0); }
    Marshal.WriteInt16(input, 0, 1); Marshal.WriteInt16(input, 2, 12); Marshal.WriteInt32(input, 4, 7); Marshal.WriteInt32(input, 8, 1);
    Marshal.WriteInt16(output, 0, 1); Marshal.WriteInt16(output, 2, 24);
    Marshal.WriteIntPtr(overlapped, 24, CreateEventW(IntPtr.Zero, true, false, IntPtr.Zero));
    bool done = DeviceIoControl(Held, 0x90240, input, 12, output, 24, IntPtr.Zero, overlapped);
    return !done && Marshal.GetLastWin32Error() == 997 ? "held" : "oplock refused: " + Marshal.GetLastWin32Error();
  }
}
'@
[Oplock]::Hold($env:RERAIL_TEST_HELD_FILE)
Start-Sleep 60`;
    return await holdWith(spawn("powershell", ["-NoProfile", "-NonInteractive", "-Command", script], {
      env: { ...process.env, RERAIL_TEST_HELD_FILE: file }, stdio: ["ignore", "pipe", "pipe"], windowsHide: true,
    }));
  }

  /** Resolves with the first line `holder` prints, or with why it ended first; it is ended after the test. */
  async function holdWith(holder: ChildProcess): Promise<string> {
    fileHolders.push(holder);
    let said = "";
    holder.stderr?.on("data", (chunk: Buffer) => { said += chunk.toString(); });
    const held = once(holder.stdout!, "data").then(([chunk]) => String(chunk).trim());
    const ended = once(holder, "close").then(() => `it ended: ${said.trim()}`, (error: Error) => `it did not start: ${error.message}`);
    return await Promise.race([held, ended]);
  }

  it.skipIf(process.platform === "win32")("refuses a private key other users may read, and reads one only its owner can", async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-daemon-tls-"));
    const { cert, key } = selfSignedCertificate();
    const files = { certFile: join(root, "cert.pem"), keyFile: join(root, "key.pem") };
    await writeFile(files.certFile, cert);
    await writeFile(files.keyFile, key);
    for (const mode of [0o644, 0o640, 0o604]) {
      await chmod(files.keyFile, mode);
      await expect(loadDaemonTls(files)).rejects.toThrow(files.keyFile);
    }
    await chmod(files.keyFile, 0o600);
    expect(await loadDaemonTls(files)).toEqual({ cert, key });
  });

  it("refuses a Windows private key whose ACL lets other accounts read it, which mode bits cannot show", async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-daemon-tls-"));
    const { cert, key } = selfSignedCertificate();
    const files = { certFile: join(root, "cert.pem"), keyFile: join(root, "key.pem") };
    await writeFile(files.certFile, cert);
    await writeFile(files.keyFile, key);
    // What a key under C:\certs inherits: Users may read it.
    const shared = { platform: "win32" as const, acl: { otherReaders: () => ["BU"] } };
    await expect(loadDaemonTls(files, shared)).rejects.toThrow(`${files.keyFile} can be read by other accounts (BU)`);
    await expect(loadDaemonTls(files, shared)).rejects.toThrow(`icacls "${files.keyFile}" /inheritance:r`);
    expect(await loadDaemonTls(files, { platform: "win32", acl: { otherReaders: () => [] } })).toEqual({ cert, key });
    // A key whose ACL cannot be read is refused too, never served unchecked.
    const unread = { platform: "win32" as const, acl: { otherReaders: (): string[] => {
      throw new Error("Get-Acl failed");
    } } };
    await expect(loadDaemonTls(files, unread)).rejects.toThrow(`Could not read who may read ${files.keyFile}: Get-Acl failed`);
    // Only a key others can read is answered by serving this computer alone. One whose readers could not be read, a
    // missing one too, is a mistake in the command and stops the start.
    await expect(loadDaemonTls(files, shared)).rejects.toBeInstanceOf(TlsKeyExposedError);
    await expect(loadDaemonTls(files, unread)).rejects.not.toBeInstanceOf(TlsKeyExposedError);
    const missing = { ...files, keyFile: join(root, "keyy.pem") };
    await expect(loadDaemonTls(missing, { platform: "linux" })).rejects.toThrow(`Could not read who may read ${missing.keyFile}: ENOENT`);
    await expect(loadDaemonTls(missing, { platform: "linux" })).rejects.not.toBeInstanceOf(TlsKeyExposedError);
    // So is a folder others can read: it is refused for that, but this account reads no key there.
    const folder = { ...files, keyFile: root };
    await expect(loadDaemonTls(folder, shared)).rejects.toThrow(`${root} can be read by other accounts (BU)`);
    await expect(loadDaemonTls(folder, shared)).rejects.not.toBeInstanceOf(TlsKeyExposedError);
  });

  it("stops the start, as main does, for a file others can read that holds no private key", async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-daemon-tls-"));
    const certFile = join(root, "cert.pem");
    await writeFile(certFile, selfSignedCertificate().cert);
    const text = join(root, "key.txt");
    await writeFile(text, "not a key\n");
    const shared = { platform: "win32" as const, acl: { otherReaders: () => ["BU"] } };
    // The certificate, as when --tls-cert and --tls-key are swapped, and a file of text: this account reads each, and
    // neither is a key it could serve with once the key is fixed.
    for (const keyFile of [certFile, text]) {
      const refused = loadDaemonTls({ certFile, keyFile }, shared);
      await expect(refused).rejects.toThrow(`${keyFile} can be read by other accounts (BU); a TLS private key must be readable by its owner only`);
      await expect(refused).rejects.not.toBeInstanceOf(TlsKeyExposedError);
    }
  });

  it.skipIf(process.platform === "win32")("stops the start at once for a FIFO others can read, never waiting for a writer", async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-daemon-tls-"));
    const fifo = join(root, "key.pem");
    await promisify(execFile)("mkfifo", ["-m", "0644", fifo]);
    const refused = loadDaemonTls({ certFile: join(root, "cert.pem"), keyFile: fifo });
    await expect(refused).rejects.toThrow(`${fifo} is open to other users (mode 0644)`);
    await expect(refused).rejects.not.toBeInstanceOf(TlsKeyExposedError);
  }, 5_000);

  it("stops the start, with the error a private key gets, for a certificate that would not serve with a key others can read", async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-daemon-tls-"));
    const { cert, key } = selfSignedCertificate();
    const keyFile = join(root, "key.pem");
    await writeFile(keyFile, key);
    await writeFile(join(root, "cert.pem"), cert);
    await writeFile(join(root, "text.pem"), "not a certificate\n");
    await writeFile(join(root, "other.pem"), selfSignedCertificate().cert);
    const shared = { platform: "win32" as const, acl: { otherReaders: () => ["BU"] } };
    // What reading it gives, as a start with a private key reads it.
    const readError = async (path: string): Promise<string> => await readFile(path, "utf8").then(() => "read", (error: Error) => error.message);
    // A misspelled path and a folder fail to read; a file of text and the key itself hold no certificate; another key's
    // certificate is not this key's. Each is a mistake in the command, which the start names once the key is private.
    const certificates: [string, string][] = [
      [join(root, "certt.pem"), await readError(join(root, "certt.pem"))], [root, await readError(root)],
      [join(root, "text.pem"), "The TLS certificate is not a PEM X.509 certificate: "],
      [keyFile, "The TLS certificate is not a PEM X.509 certificate: "], [join(root, "other.pem"), "key values mismatch"],
    ];
    for (const [certFile, error] of certificates) {
      const refused = loadDaemonTls({ certFile, keyFile }, shared);
      await expect(refused).rejects.toThrow(error);
      await expect(refused).rejects.not.toBeInstanceOf(TlsKeyExposedError);
    }
    // A folder's read error names no file: the start names the folder, which main's message beside a private key leaves out.
    await expect(loadDaemonTls({ certFile: root, keyFile }, shared)).rejects.toThrow(`${await readError(root)} '${root}'`);
    // The certificate it would serve with once the key is private: meanwhile it serves this computer alone.
    await expect(loadDaemonTls({ certFile: join(root, "cert.pem"), keyFile }, shared)).rejects.toBeInstanceOf(TlsKeyExposedError);
  });

  it.skipIf(process.platform !== "linux")("stops the start at once, as main does, when another program holds a lease on the key others can read or on its certificate", async ({ skip }) => {
    root = await mkdtemp(join(tmpdir(), "rerail-daemon-tls-"));
    const { cert, key } = selfSignedCertificate();
    const files = { certFile: join(root, "cert.pem"), keyFile: join(root, "key.pem") };
    await writeFile(files.certFile, cert);
    await writeFile(files.keyFile, key);
    await chmod(files.keyFile, 0o644);
    for (const leased of [files.keyFile, files.certFile]) {
      const lease = await holdLease(leased);
      if (lease !== "held") skip(`python3 could not hold a lease here: ${lease}`);
      // An open that waited for the lease would hold the start past `rerail daemon start`'s 15 s wait, and the daemon
      // would serve this computer after the start reported failure. Main refuses the key without opening either file.
      const started = performance.now();
      const refused = loadDaemonTls(files);
      await expect(refused).rejects.toThrow(`${files.keyFile} is open to other users (mode 0644)`);
      await expect(refused).rejects.not.toBeInstanceOf(TlsKeyExposedError);
      expect(performance.now() - started).toBeLessThan(1_000);
      for (const holder of fileHolders.splice(0)) holder.kill();
    }
  }, 5_000);

  it.skipIf(process.platform !== "win32")("stops the start within seconds, as main does, when another program holds an oplock on the key others can read or on its certificate", async ({ skip }) => {
    root = await mkdtemp(join(tmpdir(), "rerail-daemon-tls-"));
    const { cert, key } = selfSignedCertificate();
    const files = { certFile: join(root, "cert.pem"), keyFile: join(root, "key.pem") };
    await writeFile(files.certFile, cert);
    await writeFile(files.keyFile, key);
    const shared = { platform: "win32" as const, acl: { otherReaders: () => ["BU"] } };
    for (const held of [files.keyFile, files.certFile]) {
      const oplock = await holdOplock(held);
      if (oplock !== "held") skip(`PowerShell could not hold an oplock here: ${oplock}`);
      // No open skips the wait for an oplock's holder to give up its write cache, and this one never does: the start
      // stops waiting after its bound and refuses the key as main does, which reads only the key's ACL.
      const started = performance.now();
      const refused = loadDaemonTls(files, shared);
      await expect(refused).rejects.toThrow(`${files.keyFile} can be read by other accounts (BU); a TLS private key must be readable by its owner only`);
      await expect(refused).rejects.not.toBeInstanceOf(TlsKeyExposedError);
      expect(performance.now() - started).toBeLessThan(5_000);
      for (const holder of fileHolders.splice(0)) holder.kill();
    }
  }, 30_000);

  it("reads at most a mebibyte of a key others can read and of its certificate, so a huge file gets main's refusal at once", async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-daemon-tls-"));
    const { cert, key } = selfSignedCertificate();
    const keyFile = join(root, "key.pem");
    const certFile = join(root, "cert.pem");
    await writeFile(keyFile, key);
    await writeFile(certFile, cert);
    const shared = { platform: "win32" as const, acl: { otherReaders: () => ["BU"] } };
    // A real key, then a real certificate, each followed by a hole to 1 GiB: nothing is written, yet a read of the whole
    // file takes seconds. Neither is a file a person serves with, and main refuses the key without reading either.
    for (const [name, pem] of [["key-huge.pem", key], ["cert-huge.pem", cert]] as const) {
      const huge = join(root, name);
      await writeFile(huge, pem);
      await truncate(huge, 1024 ** 3);
      const files = name === "key-huge.pem" ? { certFile, keyFile: huge } : { certFile: huge, keyFile };
      const started = performance.now();
      const refused = loadDaemonTls(files, shared);
      await expect(refused).rejects.toThrow(`${files.keyFile} can be read by other accounts (BU); a TLS private key must be readable by its owner only`);
      await expect(refused).rejects.not.toBeInstanceOf(TlsKeyExposedError);
      expect(performance.now() - started).toBeLessThan(1_000);
      await rm(huge);
    }
  }, 30_000);

  it.skipIf(process.platform === "win32")("stops the start at once for a FIFO certificate beside a key others can read", async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-daemon-tls-"));
    const keyFile = join(root, "key.pem");
    await writeFile(keyFile, selfSignedCertificate().key);
    await chmod(keyFile, 0o644);
    const fifo = join(root, "cert.pem");
    await promisify(execFile)("mkfifo", ["-m", "0644", fifo]);
    // Its read would wait for a writer: it is never read, and the start gets the key's refusal, as it does on main.
    const refused = loadDaemonTls({ certFile: fifo, keyFile });
    await expect(refused).rejects.toThrow(`${keyFile} is open to other users (mode 0644)`);
    await expect(refused).rejects.not.toBeInstanceOf(TlsKeyExposedError);
  }, 5_000);

  it("records in its state file why it serves this computer alone, for rerail status to say", async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-daemon-tls-"));
    const home = join(root, "home");
    await mkdir(home, { recursive: true });
    process.env.RERAIL_HOME = home;
    const remoteRefused = `${join(root, "key.pem")} can be read by other accounts (BU)`;
    daemons.push(await RerailDaemon.start({
      version: "0.0.0-test", databasePath: join(root, "rerail.sqlite"), workspaceRoot: root, port: 0,
      nativeHealth: () => ({ ok: true, kernelVersion: "1" }), stateFile: join(home, DAEMON_STATE_FILE), remoteRefused,
    }));
    expect(await readDaemonState(join(home, DAEMON_STATE_FILE))).toMatchObject({ url: expect.stringMatching(/^http:\/\/127\.0\.0\.1:\d+\/$/u), remoteRefused });
  });

  it("names a refused key's path in its fix as it is, a `$&` in it too", async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-daemon-tls-"));
    const { cert, key } = selfSignedCertificate();
    // In a replacement string, `$&` stands for the text replaced.
    await mkdir(join(root, "certs $& keys"));
    const files = { certFile: join(root, "certs $& keys", "cert.pem"), keyFile: join(root, "certs $& keys", "key.pem") };
    await writeFile(files.certFile, cert);
    await writeFile(files.keyFile, key);
    const shared = { platform: "win32" as const, acl: { otherReaders: () => ["BU"] } };
    await expect(loadDaemonTls(files, shared)).rejects.toThrow(`icacls "${files.keyFile}" /inheritance:r`);
  });

  it("answers Rerail's own client over https pinned to the certificate it recorded, and nothing pinned to another", async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-daemon-tls-"));
    const certificate = selfSignedCertificate();
    const { state } = await startHttps(certificate);
    expect(state.url).toMatch(/^https:\/\/127\.0\.0\.1:\d+\/$/u);
    expect(state.tlsFingerprint256).toBe(certificate.fingerprint256);

    expect(await probeDaemon(state)).toMatchObject({ status: "ok", health: { pid: process.pid } });
    expect(await probeDaemon({ ...state, tlsFingerprint256: selfSignedCertificate().fingerprint256 })).toEqual({ status: "unreachable" });
    const { tlsFingerprint256: _pinned, ...unpinned } = state;
    expect(await probeDaemon(unpinned)).toEqual({ status: "unreachable" });
    // A client trusting only the system's CAs refuses the self-signed certificate: the daemon really speaks TLS.
    await expect(fetch(new URL("/healthz", state.url))).rejects.toThrow();
  });

  it("relays the MCP bridge to an https daemon", async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-daemon-tls-"));
    const { home } = await startHttps(selfSignedCertificate());
    const stdout = new PassThrough();
    const lines: string[] = [];
    stdout.on("data", (chunk: Buffer) => lines.push(...chunk.toString().split("\n").filter(line => line !== "")));
    await runRerailMcpBridge({
      home, cwd: root, stdout, stderr: new PassThrough(),
      input: Readable.from([`${JSON.stringify({ jsonrpc: "2.0", id: 1, method: "ping" })}\n`]),
    });
    expect(lines.map(line => JSON.parse(line) as unknown)).toEqual([{ jsonrpc: "2.0", id: 1, result: {} }]);
  });
});

describe("daemonFetch over https", () => {
  it("sends a request only to the certificate it pins, and nothing, capability included, to another", async () => {
    const certificate = selfSignedCertificate();
    const received: (string | undefined)[] = [];
    const server = createHttpsServer({ cert: certificate.cert, key: certificate.key }, (request, response) => {
      received.push(request.headers["x-rerail-access"] as string | undefined);
      request.resume();
      request.on("end", () => response.writeHead(200, { "content-type": "application/json" }).end(JSON.stringify({ method: request.method })));
    });
    await once(server.listen(0, "127.0.0.1"), "listening");
    try {
      const url = `https://127.0.0.1:${(server.address() as AddressInfo).port}/`;
      const init = { method: "POST", headers: { "x-rerail-access": "capability" }, body: "{}" };

      await expect(daemonFetch({ url, tlsFingerprint256: selfSignedCertificate().fingerprint256 })(new URL("/healthz", url), init)).rejects.toThrow();
      await expect(daemonFetch({ url })(new URL("/healthz", url), init)).rejects.toThrow();
      expect(received).toEqual([]);

      const response = await daemonFetch({ url, tlsFingerprint256: certificate.fingerprint256 })(new URL("/healthz", url), init);
      expect({ status: response.status, body: await response.json() as unknown }).toEqual({ status: 200, body: { method: "POST" } });
      expect(received).toEqual(["capability"]);
    } finally {
      server.closeAllConnections();
      await once(server.close(), "close");
    }
  });
});
