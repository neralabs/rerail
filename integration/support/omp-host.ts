import { spawn } from "node:child_process";
import type { ChildProcessWithoutNullStreams } from "node:child_process";
import { access, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { constants } from "node:fs";
import { tmpdir } from "node:os";
import { isAbsolute, join, resolve } from "node:path";
import { createInterface } from "node:readline";
import { setTimeout as delay } from "node:timers/promises";
import { stopDaemon } from "../src/daemon-control.js";
import { seedContractCodeRule } from "./contract-code-rule.js";
import { startTestRepository } from "../test/support/started-repository.js";
import type { OmpSessionEntryLike } from "../src/omp-semantic.js";

export interface HostResponse {
  content: Array<string | { type: "toolCall"; name: string; arguments: Record<string, unknown> }>;
}
export interface HostEvidence {
  sessionId: string;
  branch: OmpSessionEntryLike[];
  calls: Array<{ context: { messages: unknown[] } }>;
  notificationStarted: boolean;
  notificationReleased: boolean;
  auth?: { type: string; accessHash: string; refresh: string; expires: number; accountId: string };
}
type Frame = { type: string; id?: string; success?: boolean; error?: string; [key: string]: unknown };

/** A real standalone host. No SDK runtime crosses into the Node test process. */
export class OmpHost {
  readonly frames: Frame[] = [];
  readonly errors: string[] = [];
  private closed = false;
  private exitCode: number | null = null;
  private failure?: Error;
  private sequence = 0;
  private constructor(readonly cwd: string, private readonly child: ChildProcessWithoutNullStreams, private readonly temporaryDirectory: string) {
    createInterface({ input: child.stdout }).on("line", line => {
      try { this.frames.push(JSON.parse(line) as Frame); }
      catch { this.errors.push(line); }
    });
    child.stderr.on("data", chunk => this.errors.push(String(chunk)));
    child.on("error", error => { this.failure = error; });
    child.on("close", code => { this.closed = true; this.exitCode = code; });
  }
  static async start(cwd: string, responses: HostResponse[], options: { extension?: "default" | "engine" | "none"; authPath?: string } = {}): Promise<OmpHost> {
    const executable = process.env.RERAIL_OMP_EXECUTABLE;
    if (!executable || !isAbsolute(executable)) {
      throw new Error("Real standalone OMP required: set RERAIL_OMP_EXECUTABLE to its absolute executable path");
    }
    await access(executable, constants.X_OK);
    const root = resolve(".");
    const fixture = join(root, "node_modules/.cache/rerail/omp-host.mjs");
    await access(fixture);
    const config = join(cwd, "host-config.json");
    await mkdir(join(cwd, "host-home"), { recursive: true });
    await mkdir(join(cwd, "host-agent"), { recursive: true });
    const extension = options.extension ?? "default";
    if (extension !== "none") seedContractCodeRule(join(cwd, "home"), cwd);
    startTestRepository(cwd, join(cwd, "home", "rerail.sqlite"));
    await writeFile(config, JSON.stringify({ responses, engine: extension === "engine",
      extension: extension === "none" ? undefined : join(root, "dist", "default-extension.js"),
      modelPath: join(root, "model"), evidencePath: join(cwd, "host-evidence.json"), authPath: options.authPath,
    }));
    const temporaryDirectory = await mkdtemp(join(process.platform === "win32" ? tmpdir() : "/tmp", "rerail-omp-test-"));
    // Provider/model are selected after `ready`: CLI --provider/--model resolve before extensions load, so the fixture provider is unknown there.
    let child: ChildProcessWithoutNullStreams;
    try {
      child = spawn(executable, ["--mode", "rpc", "--no-session", "--no-extensions", "--no-skills", "--no-rules", "--no-lsp", "--no-pty",
        "--extension", fixture, "--thinking", "off"], {
        cwd, stdio: "pipe", env: { PATH: process.env.PATH, TMPDIR: temporaryDirectory, HOME: join(cwd, "host-home"), PI_CODING_AGENT_DIR: join(cwd, "host-agent"),
          RERAIL_HOME: join(cwd, "home"), RERAIL_PACKAGE_ROOT: root, RERAIL_OMP_TEST_CONFIG: config },
      });
    } catch (error) {
      await rm(temporaryDirectory, { recursive: true, force: true });
      throw error;
    }
    const host = new OmpHost(cwd, child, temporaryDirectory);
    try {
      await host.waitFor(frame => frame.type === "ready");
      // Fails, rather than silently falling back to a configured paid model, if the fixture provider did not load.
      await host.command({ type: "set_model", provider: "rerail-test", modelId: "rerail-script" });
      return host;
    } catch (error) { await host.close(); throw error; }
  }
  private async waitFor(predicate: (frame: Frame) => boolean, from = 0): Promise<Frame> {
    const deadline = Date.now() + 60_000;
    while (Date.now() < deadline) {
      const found = this.frames.slice(from).find(predicate);
      if (found) return found;
      if (this.failure) throw this.failure;
      if (this.closed) throw new Error(`OMP exited (${this.exitCode}): ${this.errors.join("")}`);
      await delay(10);
    }
    throw new Error(`Standalone OMP timed out: ${this.errors.join("")}\n${JSON.stringify(this.frames.slice(-5))}`);
  }
  async command(command: Record<string, unknown>): Promise<Frame> {
    const id = String(++this.sequence);
    this.child.stdin.write(`${JSON.stringify({ ...command, id })}\n`);
    const response = await this.waitFor(frame => frame.type === "response" && frame.id === id);
    if (!response.success) throw new Error(`OMP ${String(command.type)} failed: ${response.error}`);
    return response;
  }
  async prompt(message: string): Promise<void> {
    const from = this.frames.length;
    const response = await this.command({ type: "prompt", message });
    await this.waitFor(frame => frame.type === "agent_end" || (frame.type === "prompt_result" && frame.id === response.id), from);
    // agent_end observers and queued steers may still be settling; the command's idle barrier handles both.
    await this.snapshot();
  }
  async snapshot(): Promise<HostEvidence> {
    const from = this.frames.length;
    const response = await this.command({ type: "prompt", message: "/rerail-test-snapshot" });
    await this.waitFor(frame => frame.type === "prompt_result" && frame.id === response.id, from);
    return JSON.parse(await readFile(join(this.cwd, "host-evidence.json"), "utf8")) as HostEvidence;
  }
  async append(customType: string, data: unknown): Promise<void> {
    const from = this.frames.length;
    const response = await this.command({ type: "prompt", message: `/rerail-test-append ${JSON.stringify({ customType, data })}` });
    await this.waitFor(frame => frame.type === "prompt_result" && frame.id === response.id, from);
  }
  async close(): Promise<void> {
    try {
      if (!this.closed) {
        this.child.stdin.end();
        const deadline = Date.now() + 15_000;
        while (!this.closed && Date.now() < deadline) await delay(10);
        if (!this.closed) {
          this.child.kill("SIGKILL");
          while (!this.closed) await delay(10);
          throw new Error(`OMP did not shut down cleanly: ${this.errors.join("")}`);
        }
        if (this.exitCode !== 0) throw new Error(`OMP exited (${this.exitCode}): ${this.errors.join("")}`);
      }
    } finally {
      try { await stopDaemon(join(this.cwd, "home")); }
      finally { await rm(this.temporaryDirectory, { recursive: true, force: true }); }
    }
  }
}
