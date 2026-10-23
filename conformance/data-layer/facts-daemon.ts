/**
 * A client of the facts daemon (`facts-a daemon`, docs/facts/PROTOCOL.md): starts the daemon, speaks its framed protocol
 * over the Unix socket, matches replies to requests by `seq` (a request's partial frames go to its listener before the
 * reply) and hands cross-file pushes to a listener. The end-to-end run of the memory47 harness
 * (`benchmark/workspaces/workspaces.ts run --engine facts`) drives it.
 */
import { type ChildProcess, spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, mkdtempSync } from "node:fs";
import { connect, type Socket } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";

export type DaemonMessage = Record<string, unknown> & { op: string };

export interface DaemonOptions {
  binary: string;
  policies: string;
  store: string;
  threads?: number;
  cross?: string;
  /** Where the daemon's stderr goes (inherited when absent). */
  log?: number;
}

/** A Git blob id, as the enforcer hashes a file (`blob <length>\0` + bytes, SHA-1). */
export function gitBlobId(bytes: Buffer): string {
  return createHash("sha1").update(`blob ${bytes.length}\0`).update(bytes).digest("hex");
}

interface Pending {
  resolve: (message: DaemonMessage) => void;
  partial: (message: DaemonMessage) => void;
}

export class FactsDaemon {
  readonly pid: number;
  #next = 1;
  #pending = new Map<number, Pending>();
  #buffer = Buffer.alloc(0);
  #onPush: (message: DaemonMessage) => void = () => {};

  private constructor(private readonly child: ChildProcess, private readonly socket: Socket) {
    this.pid = child.pid!;
    socket.on("data", chunk => this.#receive(chunk));
  }

  static async start(options: DaemonOptions): Promise<FactsDaemon> {
    const path = join(mkdtempSync(join(tmpdir(), "rerail-facts-daemon-")), "daemon.sock");
    const args = ["daemon", "--socket", path, "--policies", options.policies, "--store", options.store, ...(options.threads === undefined ? [] : ["--threads", String(options.threads)]), ...(options.cross === undefined ? [] : ["--cross", options.cross])];
    const child = spawn(options.binary, args, { stdio: ["ignore", "inherit", options.log ?? "inherit"] });
    let exited: number | null = null;
    child.on("exit", code => { exited = code ?? -1; });
    while (!existsSync(path)) {
      if (exited !== null) throw new Error(`the facts daemon exited with ${exited} before listening`);
      await sleep(20);
    }
    const connected = Promise.withResolvers<void>();
    const socket = connect(path, () => connected.resolve());
    socket.once("error", connected.reject);
    await connected.promise;
    return new FactsDaemon(child, socket);
  }

  /** Cross-file pushes (`op: findings`, `cross: true`) arrive here. */
  onPush(listener: (message: DaemonMessage) => void): void {
    this.#onPush = listener;
  }

  /** One request; its partial frames (`partial: true`, same `seq`) go to `partial` as they arrive, then the reply resolves. */
  request(header: Record<string, unknown>, body: Buffer = Buffer.alloc(0), partial: (message: DaemonMessage) => void = () => {}): Promise<DaemonMessage> {
    const seq = this.#next++;
    const h = Buffer.from(JSON.stringify({ ...header, seq }));
    const frame = Buffer.alloc(8 + h.length + body.length);
    frame.writeUInt32LE(h.length, 0);
    h.copy(frame, 4);
    frame.writeUInt32LE(body.length, 4 + h.length);
    body.copy(frame, 8 + h.length);
    const reply = Promise.withResolvers<DaemonMessage>();
    this.#pending.set(seq, { resolve: message => (message.op === "error" ? reply.reject(new Error(`facts daemon: ${String(message.error)}`)) : reply.resolve(message)), partial });
    this.socket.write(frame);
    return reply.promise;
  }

  /**
   * An edit by Git blob id, hash first: the bytes go only when the daemon answers that it does not know the content.
   * Returns the findings reply, whether the bytes were sent, and the partial frames before the reply.
   */
  async edit(worktree: string, path: string, bytes: Buffer | null, partial: (message: DaemonMessage) => void = () => {}): Promise<{ reply: DaemonMessage; sentBytes: boolean }> {
    if (bytes === null) return { reply: await this.request({ op: "edit", worktree, path }, undefined, partial), sentBytes: false };
    const git = gitBlobId(bytes);
    const first = await this.request({ op: "edit", worktree, path, git }, undefined, partial);
    if (first.op !== "need_bytes") return { reply: first, sentBytes: false };
    return { reply: await this.request({ op: "edit", worktree, path, git }, bytes, partial), sentBytes: true };
  }

  /** Shuts the daemon down; with `attribute`, its `bye` carries the memory attribution (`Engine::attribute`). */
  async close(attribute = false): Promise<DaemonMessage> {
    const bye = await this.request({ op: "shutdown", attribute });
    this.socket.end();
    if (this.child.exitCode === null) {
      const exited = Promise.withResolvers<void>();
      this.child.once("exit", () => exited.resolve());
      await exited.promise;
    }
    return bye;
  }

  #receive(chunk: Buffer): void {
    this.#buffer = Buffer.concat([this.#buffer, chunk]);
    while (this.#buffer.length >= 4) {
      const headerLength = this.#buffer.readUInt32LE(0);
      if (this.#buffer.length < 8 + headerLength) return;
      const bodyLength = this.#buffer.readUInt32LE(4 + headerLength);
      if (this.#buffer.length < 8 + headerLength + bodyLength) return;
      const message = JSON.parse(this.#buffer.subarray(4, 4 + headerLength).toString("utf8")) as DaemonMessage;
      this.#buffer = this.#buffer.subarray(8 + headerLength + bodyLength);
      const seq = typeof message.seq === "number" ? message.seq : undefined;
      const pending = seq === undefined ? undefined : this.#pending.get(seq);
      if (pending === undefined) {
        this.#onPush(message);
        continue;
      }
      if (message.partial === true) {
        pending.partial(message);
        continue;
      }
      this.#pending.delete(seq!);
      pending.resolve(message);
    }
  }
}
