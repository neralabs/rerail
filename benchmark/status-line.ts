// `just rerail-status-line`: run the built Claude Code status line (dist/hosts/claude/claude-status.js) the way Claude
// Code runs it, every few seconds in each session, against a stand-in daemon in this process, and report what one run
// costs: wall time, its own CPU time and peak memory, and how many modules it loads, beside Node with nothing to do.
// `--shell` starts it through the shell Claude Code uses (Git Bash on Windows, sh elsewhere), as Claude Code does.
// Nothing is written outside a temporary folder.
import { spawn } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { createServer, type Server } from "node:http";
import { tmpdir } from "node:os";
import { dirname, join, resolve, win32 } from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";
import { presentationResponseSchema } from "../src/api/index.js";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const USAGE = "usage: status-line [--runs N] [--shell] [--script <built claude-status.js>]";

interface Options { runs: number; shell: boolean; script: string }

function options(): Options {
  const { values } = parseArgs({
    options: {
      runs: { type: "string", default: "20" },
      shell: { type: "boolean", default: false },
      script: { type: "string", default: join(ROOT, "dist", "hosts", "claude", "claude-status.js") },
    },
  });
  const runs = Number(values.runs);
  if (!Number.isSafeInteger(runs) || runs < 1) throw new Error(`--runs must be a positive integer\n${USAGE}`);
  const script = resolve(values.script);
  if (!existsSync(script)) throw new Error(`${script} is missing; run npm run build:js first\n${USAGE}`);
  return { runs, shell: values.shell, script };
}

/** What a run reports of itself at exit: its CPU time, peak resident memory and the modules it loaded. */
interface RunReport { wallMs: number; cpuMs: number; maxRssMb: number; modules: number }

const PRELOAD = `
import { registerHooks } from "node:module";
let modules = 0;
registerHooks({ load(url, context, next) { modules += 1; return next(url, context); } });
process.on("exit", () => {
  const usage = process.resourceUsage();
  process.stderr.write("\\n@@status-line " + JSON.stringify({
    cpuMs: (usage.userCPUTime + usage.systemCPUTime) / 1000, maxRssMb: usage.maxRSS / 1024, modules,
  }) + "\\n");
});
`;
const PRELOAD_URL = `data:text/javascript;base64,${Buffer.from(PRELOAD).toString("base64")}`;

/** The shell Claude Code starts the status line in: Git Bash beside Git on Windows, sh elsewhere. */
function claudeShell(): string {
  if (process.platform !== "win32") return "/bin/sh";
  const named = process.env.CLAUDE_CODE_GIT_BASH_PATH;
  const candidates = [named, win32.join(process.env.ProgramFiles ?? String.raw`C:\Program Files`, "Git", "bin", "bash.exe")];
  const bash = candidates.find(candidate => candidate !== undefined && candidate !== "" && existsSync(candidate));
  if (bash === undefined) throw new Error("--shell needs Git Bash on Windows (CLAUDE_CODE_GIT_BASH_PATH)");
  return bash;
}

const quote = (value: string): string => `'${value.replaceAll("'", `'\\''`)}'`;

/** One run with `input` on stdin; `args` follow the preload on node's command line. */
async function run(args: readonly string[], input: string, env: NodeJS.ProcessEnv, shell: boolean): Promise<RunReport> {
  const nodeArgs = [`--import=${PRELOAD_URL}`, ...args];
  const [file, argv] = shell
    ? [claudeShell(), ["-c", [process.execPath, ...nodeArgs].map(quote).join(" ")]]
    : [process.execPath, nodeArgs];
  const started = performance.now();
  const child = spawn(file, argv, { env, stdio: ["pipe", "ignore", "pipe"], windowsHide: true });
  let stderr = "";
  child.stderr.setEncoding("utf8").on("data", (chunk: string) => { stderr += chunk; });
  child.stdin.end(input);
  const code = await new Promise<number | null>((done, failed) => { child.once("error", failed); child.once("close", done); });
  const wallMs = performance.now() - started;
  const line = /@@status-line (\{.*\})/u.exec(stderr)?.[1];
  if (code !== 0 || line === undefined) throw new Error(`run exited ${code}: ${stderr.trim()}`);
  const report = JSON.parse(line) as Omit<RunReport, "wallMs">;
  return { wallMs, ...report };
}

const median = (values: readonly number[]): number => [...values].sort((a, b) => a - b)[Math.floor(values.length / 2)]!;
const percentile = (values: readonly number[], p: number): number => [...values].sort((a, b) => a - b)[Math.min(values.length - 1, Math.floor(values.length * p))]!;

function summary(name: string, reports: readonly RunReport[]): string {
  const column = (pick: (report: RunReport) => number, unit: string): string =>
    `${median(reports.map(pick)).toFixed(1).padStart(7)}${percentile(reports.map(pick), 0.9).toFixed(1).padStart(7)} ${unit}`;
  return `  ${name.padEnd(16)}wall${column(report => report.wallMs, "ms")}   cpu${column(report => report.cpuMs, "ms")}   `
    + `peak rss${column(report => report.maxRssMb, "MB")}   modules ${median(reports.map(report => report.modules))}`;
}

async function main(): Promise<void> {
  const settings = options();
  const root = mkdtempSync(join(tmpdir(), "rerail-status-line-"));
  let server: Server | undefined;
  try {
    const home = join(root, "home");
    const rerailHome = join(home, ".rerail");
    const checkout = join(root, "checkout");
    mkdirSync(rerailHome, { recursive: true });
    mkdirSync(checkout);
    const cwd = realpathSync(checkout);
    // The answer a started checkout's live daemon gives for a recorded session, checked against the daemon's contract.
    const answer = presentationResponseSchema.parse({
      presence: "compact", repositoryState: "started", scope: { repositoryKey: "clone:git:benchmark", checkoutPath: cwd },
      context: {
        daemon: { pid: process.pid, database: { path: join(rerailHome, "rerail.sqlite"), exists: true }, native: { ok: true }, workspaceRoot: cwd },
        session: {
          session: { sessionId: "benchmark", host: "claude-code", cwd, repositoryId: "clone:git:benchmark", startedAt: 1, endedAt: null, eventCount: 12 },
          hud: { activeRuleCount: 417, steeringCount: 3, overlays: [], awaitingApproval: 0 },
          audit: null,
        },
      },
    });
    let requests = 0;
    server = createServer((request, response) => {
      requests += 1;
      if (request.url?.startsWith("/api/presentation") === true) {
        response.writeHead(200, { "content-type": "application/json" }).end(JSON.stringify(answer));
        return;
      }
      response.writeHead(404).end();
    });
    const listening = Promise.withResolvers<void>();
    server.listen(0, "127.0.0.1", listening.resolve);
    await listening.promise;
    const address = server.address();
    if (address === null || typeof address === "string") throw new Error("no port");
    writeFileSync(join(rerailHome, "daemon.json"), JSON.stringify({
      pid: process.pid, host: "127.0.0.1", port: address.port, url: `http://127.0.0.1:${address.port}/`, version: "benchmark",
      databasePath: join(rerailHome, "rerail.sqlite"), ui: "bundled", startedAt: Date.now(), packageRoot: ROOT,
      kernelSocketPath: process.platform === "win32" ? String.raw`\\.\pipe\rerail-status-line-benchmark` : join(root, "engine.sock"),
    }));
    // No previous status line is registered in this home, so a run is Rerail's part alone.
    const env = { ...process.env, RERAIL_HOME: rerailHome, HOME: home, USERPROFILE: home };
    const input = JSON.stringify({ session_id: "benchmark", cwd, workspace: { current_dir: cwd, project_dir: cwd }, model: { id: "model" } });
    const status: RunReport[] = [];
    const floor: RunReport[] = [];
    // Alternated, so whatever else the machine does weighs on both alike.
    for (let index = 0; index < settings.runs; index += 1) {
      status.push(await run([settings.script], input, env, settings.shell));
      floor.push(await run(["-e", "0"], "", env, settings.shell));
    }
    console.log(`${settings.runs} runs each${settings.shell ? ` through ${claudeShell()}` : ""} (median, p90); ${requests} daemon reads`);
    console.log(summary("status line", status));
    console.log(summary("node -e 0", floor));
    console.log(`  Rerail's part   wall ${(median(status.map(r => r.wallMs)) - median(floor.map(r => r.wallMs))).toFixed(1)} ms, `
      + `cpu ${(median(status.map(r => r.cpuMs)) - median(floor.map(r => r.cpuMs))).toFixed(1)} ms per run; `
      + `at Claude Code's 5 s refresh that is ${((median(status.map(r => r.cpuMs)) * 12) / 600).toFixed(2)}% of a core per session`);
  } finally {
    if (server !== undefined) {
      const closed = Promise.withResolvers<void>();
      server.close(() => closed.resolve());
      await closed.promise;
    }
    rmSync(root, { recursive: true, force: true });
  }
}

await main();
