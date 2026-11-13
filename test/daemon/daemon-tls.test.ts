import { once } from "node:events";
import { chmod, mkdir, mkdtemp, writeFile } from "node:fs/promises";
import { createServer as createHttpsServer } from "node:https";
import type { AddressInfo } from "node:net";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { PassThrough, Readable } from "node:stream";
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
  const originalRerailHome = process.env.RERAIL_HOME;

  afterEach(async () => {
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
  });

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
