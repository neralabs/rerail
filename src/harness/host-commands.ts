import { spawn } from "node:child_process";
import { join } from "node:path";
import { ENTRY_POINTS } from "../engine/package-root.js";
import { readPresentation, readRulesDashboard, readSummaryState } from "../presentation/session-presentation.js";
import type { RepositoryStartState } from "../engine/catalog/repository-model.js";
import { nodeExecutable, openDashboard, type PageOpening } from "../runtime/daemon-control.js";
import { createRuleInstructions, RERAIL_COMMANDS, showMeInstructions, type RerailCommandName } from "./rerail-commands.js";
import { renderRerailSummary } from "./rerail-summary.js";
import { repositoryName } from "../engine/catalog/repository-identity.js";
import { pickSetup, type SetupStepRunner } from "./setup-picker.js";
import { setupStepSchema } from "./setup-step.js";

/** Minimal command-host surface shared by OMP and Pi. */
export interface HostCommandHost {
  registerCommand(name: string, command: {
    description: string;
    handler(args: string, context: HostCommandContext): Promise<void>;
  }): void;
  /** Hands a prompt to the coding agent as if the person had typed it; `/rerail-create` runs through it. */
  sendUserMessage(content: string): void;
}

export interface HostCommandContext {
  hasUI?: boolean;
  ui: {
    notify(message: string, level: "info" | "error"): void;
    select(title: string, options: string[]): Promise<string | undefined>;
    input(title: string, placeholder?: string): Promise<string | undefined>;
    confirm(title: string, message: string): Promise<boolean>;
  };
}

export interface HostCommandOptions {
  /** Installed package root; the launcher runs `<packageRoot>/dist/cli/cli.js`. */
  packageRoot: string;
  host?: "omp" | "pi";
  cwd(): string;
  /** Current host session, so `/rerail` reads this exact session and never guesses. */
  sessionId?(): string | undefined;
  /** Release the in-process runtime's database handles before Rerail is removed from this computer. */
  detach?(): Promise<void>;
  /** Rebuild the runtime after start or stop, or after a removal that failed, so it runs only where Rerail is started. */
  reattach?(): Promise<void>;
  /** A database configured in place of the Rerail home's; start and stop change it, as the extension's gate reads it. */
  databasePath?(): string | undefined;
}

/** Bytes of combined stdout+stderr kept for the notification; the rest is dropped. */
const OUTPUT_CAP = 10 * 1024 * 1024;

interface LauncherResult {
  code: number;
  stdout: string;
  stderr: string;
}

/** Start, stop and state change or read the database the extension's gate reads, when one was configured. */
function databaseArgs(options: HostCommandOptions): string[] {
  const database = options.databasePath?.();
  return database === undefined ? [] : ["--database", database];
}

async function runLauncher(options: HostCommandOptions, args: readonly string[], hostArgs: readonly string[] = []): Promise<LauncherResult> {
  const child = spawn(nodeExecutable(), [join(options.packageRoot, ENTRY_POINTS.cli), ...args, ...hostArgs], {
    cwd: options.cwd(),
    env: process.env,
    stdio: ["ignore", "pipe", "pipe"],
  });
  const { promise, resolve, reject } = Promise.withResolvers<LauncherResult>();
  const output = { stdout: "", stderr: "" };
  const append = (stream: keyof typeof output, chunk: string): void => {
    if (output.stdout.length + output.stderr.length >= OUTPUT_CAP) return;
    output[stream] += chunk.slice(0, OUTPUT_CAP - output.stdout.length - output.stderr.length);
  };
  child.stdout.setEncoding("utf8");
  child.stderr.setEncoding("utf8");
  child.stdout.on("data", chunk => append("stdout", chunk));
  child.stderr.on("data", chunk => append("stderr", chunk));
  child.once("error", reject);
  child.once("close", (code, signal) => resolve({ code: signal === null ? code ?? 1 : 1, ...output }));
  return promise;
}

const START_STATES: readonly RepositoryStartState[] = ["started", "stopped", "not-set-up"];

/** `rerail state` for this checkout, as `/rerail-start` reads it; OMP runs on Bun, which cannot open the database itself. */
async function launcherStartState(options: HostCommandOptions): Promise<RepositoryStartState> {
  const result = await runLauncher(options, ["state"], databaseArgs(options));
  const state = result.stdout.trim();
  const known = START_STATES.find(candidate => candidate === state);
  if (result.code !== 0 || known === undefined) throw new Error(`rerail state failed: ${(result.stdout + result.stderr).trim()}`);
  return known;
}

function notifyResult(ctx: HostCommandContext, name: string, result: LauncherResult): void {
  ctx.ui.notify((result.stdout + result.stderr).trim() || `/${name} exited with code ${result.code}.`, result.code === 0 ? "info" : "error");
}

/** The notice for a page that was just opened; an Orca tab says so, the desktop browser needs no qualifier. */
function openingNotice(url: string, opening: PageOpening): string {
  return opening === "orca-tab" ? `Opening ${url} in the Orca window` : `Opening ${url}`;
}

/**
 * Each set-up step runs through the launcher's `--json`, in this session's folder. That is this session's package,
 * which a `/rerail-update` since may have replaced; set-up's steps never install it over the active one.
 */
function setupSteps(options: HostCommandOptions): SetupStepRunner {
  const cwd = options.cwd();
  const launcher = { ...options, cwd: () => cwd };
  return async args => {
    const result = await runLauncher(launcher, args, databaseArgs(options));
    try {
      return setupStepSchema.parse(JSON.parse(result.stdout));
    } catch {
      throw new Error((result.stdout + result.stderr).trim() || `Set-up exited with code ${result.code}.`);
    }
  };
}

/**
 * `/rerail-start` from OMP or Pi: where the repository is not set up, set-up's questions asked through the coding
 * agent's own `ctx.ui.select` / `ctx.ui.input`, each answer run as set-up's `--json` step (`pickSetup`); where it
 * is, the first step starts Rerail and says so. The picker shows every screen itself. Without a UI (print mode) the
 * launcher's text says what to run instead. Returns whether Rerail ended up started or stopped, so the runtime
 * re-attaches.
 */
async function startFromCommand(options: HostCommandOptions, ctx: HostCommandContext): Promise<boolean> {
  if (ctx.hasUI === false) {
    const result = await runLauncher(options, ["start"], databaseArgs(options));
    notifyResult(ctx, "rerail-start", result);
    return result.code === 0;
  }
  const last = await pickSetup(ctx.ui, setupSteps(options));
  return last !== undefined && last.step !== "error";
}

/** A POSIX single-quoted shell word, for the launcher command the coding agent runs from `/rerail-create`. */
const shellWord = (value: string): string => "'" + value.replaceAll("'", String.raw`'\''`) + "'";

const REMOVE = "Yes, remove Rerail";
const STOP_HERE = "Just stop it in this repo (/rerail-stop)";

type CommandAction = (ctx: HostCommandContext, args: string) => Promise<void>;

export function registerHostCommands(pi: HostCommandHost, options: HostCommandOptions): void {
  function register(name: RerailCommandName, action: CommandAction, acceptsArguments = false): void {
    pi.registerCommand(name, {
      description: RERAIL_COMMANDS.find(command => command.name === name)?.description ?? name,
      async handler(args, ctx) {
        if (!acceptsArguments && args.trim() !== "") {
          ctx.ui.notify(`/${name} takes no arguments.`, "error");
          return;
        }
        try {
          await action(ctx, args.trim());
        } catch (error) {
          ctx.ui.notify(`/${name}: ${error instanceof Error ? error.message : String(error)}`, "error");
        }
      },
    });
  }

  register("rerail", async ctx => {
    const cwd = options.cwd();
    const host = options.host ?? "omp";
    const sessionId = options.sessionId?.();
    // Nothing autostarts where Rerail is off, so with no daemon answering the launcher says whether it is on here.
    const startState = (): Promise<RepositoryStartState> => launcherStartState(options);
    const presentation = sessionId === undefined
      ? null
      : await readPresentation({ sessionId, host, cwd, onDemand: true, startState });
    // No session is attached where Rerail is stopped or never started.
    const repositoryState = presentation === null ? await readSummaryState(cwd, host, startState) : presentation.repositoryState;
    ctx.ui.notify(renderRerailSummary({
      presentation,
      repositoryState,
      ruleCount: null,
      rulesUrl: (await readRulesDashboard({ cwd })).url,
      repositoryName: repositoryName(cwd),
      sigil: "/",
    }), "info");
  });

  register("rerail-ui", async ctx => {
    const page = await readRulesDashboard({ cwd: options.cwd() });
    // The page needs a reachable daemon; the reason says why there is none.
    if (page.url === null) throw new Error(`no dashboard to open: ${page.reason}`);
    ctx.ui.notify(openingNotice(page.url, await openDashboard(page.url)), "info");
  });

  // The launcher, as the coding agent runs it from `/rerail-create` and `/rerail-show-me`.
  const cli = `${shellWord(nodeExecutable())} ${shellWord(join(options.packageRoot, ENTRY_POINTS.cli))}`;

  register("rerail-create", async (_ctx, request) => {
    pi.sendUserMessage(createRuleInstructions({ cli, packageRoot: options.packageRoot, request: request === "" ? "(none given)" : `"${request}"` }));
  }, true);

  register("rerail-show-me", async () => {
    pi.sendUserMessage(showMeInstructions({ cli, sigil: "/", write: "edit-tool" }));
  });

  // Start and stop run the launcher, so the CLI owns the wording; without a terminal it addresses the coding agent's user.
  // Re-attaching attaches the runtime where Rerail is started, and leaves only the "not on here" footer where it is not.
  register("rerail-start", async ctx => {
    if (await startFromCommand(options, ctx)) await options.reattach?.();
  });
  register("rerail-stop", async ctx => {
    const result = await runLauncher(options, ["stop"], databaseArgs(options));
    if (result.code === 0) await options.reattach?.();
    notifyResult(ctx, "rerail-stop", result);
  });

  register("rerail-update", async ctx => {
    const result = await runLauncher(options, ["update"]);
    notifyResult(ctx, "rerail-update", result.code === 0
      ? { ...result, stdout: `${result.stdout.trimEnd()}\nNew sessions use the new version.\n` }
      : result);
  });

  // Design A9/A10: say what goes and what stays, ask once, and remove only on an explicit yes.
  register("rerail-rage-quit", async ctx => {
    const choice = await ctx.ui.select([
      "This removes Rerail from this computer, from every coding agent, in every repo.",
      "Your rules and history are kept in a backup folder.",
      "",
      "Remove Rerail from this computer? You can install it again later and get your rules back.",
    ].join("\n"), [REMOVE, STOP_HERE, "Cancel"]);
    if (choice === STOP_HERE) {
      const result = await runLauncher(options, ["stop"], databaseArgs(options));
      if (result.code === 0) await options.reattach?.();
      notifyResult(ctx, "rerail-stop", result);
      return;
    }
    if (choice !== REMOVE) return;
    await options.detach?.();
    // A removal that fails leaves Rerail installed and on, so the session gets its rule checks back.
    let result: LauncherResult;
    try {
      result = await runLauncher(options, ["install", "--remove"]);
    } catch (error) {
      await options.reattach?.();
      throw error;
    }
    if (result.code !== 0) await options.reattach?.();
    notifyResult(ctx, "rerail-rage-quit", result);
  });
}
