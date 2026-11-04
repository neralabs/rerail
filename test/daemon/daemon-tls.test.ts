import { once } from "node:events";
import { chmod, mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { createServer as createHttpsServer } from "node:https";
import type { AddressInfo } from "node:net";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { PassThrough, Readable } from "node:stream";
import { afterEach, describe, expect, it } from "vitest";
import { runRerailMcpBridge } from "../../src/api/index.js";
import {
  DAEMON_TRANSPORT_FLAGS,
  DAEMON_TRANSPORT_OPTIONS,
  loadDaemonTls,
  parseArguments,
  parseDaemonTransport,
  RerailDaemon,
} from "../../src/daemon/daemon.js";
import { DAEMON_STATE_FILE, daemonFetch, probeDaemon, readDaemonState, type DaemonState } from "../../src/runtime/index.js";
import { selfSignedCertificate } from "../support/self-signed-certificate.js";

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
});

describe("TLS daemon", () => {
  let root: string;
  const daemons: RerailDaemon[] = [];
  const originalRerailHome = process.env.RERAIL_HOME;

  afterEach(async () => {
    await Promise.all(daemons.splice(0).map(daemon => daemon.close()));
    if (originalRerailHome === undefined) delete process.env.RERAIL_HOME;
    else process.env.RERAIL_HOME = originalRerailHome;
    await rm(root, { recursive: true, force: true });
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
