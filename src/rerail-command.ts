import { spawn } from "node:child_process";
import { join } from "node:path";
import { readPresentation, readRulesPage } from "./session-presentation.js";
import { nodeExecutable, openPage, type PageOpening } from "./daemon-control.js";
import { policyControlsSchema } from "./scan-control-schema.js";
import type { PolicyControls, PolicySettings } from "./scan-control-schema.js";

/** Minimal command-host surface shared by OMP and Pi. */
export interface RerailCommandHost {
  registerCommand(name: string, command: {
    description: string;
    handler(args: string, context: RerailCommandContext): Promise<void>;
  }): void;
}

export interface RerailCommandContext {
  hasUI?: boolean;
  ui: {
    notify(message: string, level: "info" | "error"): void;
    select(title: string, options: string[]): Promise<string | undefined>;
    input(title: string, placeholder?: string): Promise<string | undefined>;
    confirm(title: string, message: string): Promise<boolean>;
  };
}

export interface RerailCommandOptions {
  /** Installed package root; the launcher runs `<packageRoot>/dist/cli.js`. */
  packageRoot: string;
  host?: "omp" | "pi";
  cwd(): string;
  /** Current host session, injected for scan controls so the launcher never guesses. */
  sessionId?(): string | undefined;
  /** Release the in-process runtime's database handles before a destructive launcher action. */
  detach?(): Promise<void>;
  /** Rebuild the runtime afterwards; runs whether or not the action succeeded. */
  reattach?(): Promise<void>;
  /** A database configured in place of the Rerail home's; start and stop change it, as the extension's gate reads it. */
  databasePath?(): string | undefined;
}

/** Bytes of combined stdout+stderr kept for the notification; the rest is dropped. */
const OUTPUT_CAP = 10 * 1024 * 1024;

const LAUNCHER_COMMANDS = [
  { name: "rerail-status", description: "Show whether Rerail is on in this repo, and the daemon and installed version", argv: ["status"] },
  { name: "rerail-check", description: "Check for a Rerail update without installing it", argv: ["update", "check"] },
  { name: "rerail-update", description: "Install the latest Rerail version", argv: ["update"] },
  { name: "rerail-rollback", description: "Restore the previously installed Rerail version", argv: ["update", "rollback"] },
  { name: "rerail-reset", description: "Reset this repository's Rerail session history, across all worktrees, after confirmation", argv: ["reset"] },
];

interface PolicyChoice<Value extends string = PolicySettings[keyof PolicySettings]> {
  value: Value;
  label: string;
}

const MODES: readonly PolicyChoice<PolicySettings["mode"]>[] = [
  { value: "log", label: "log — Record diagnostics without steering the agent" },
  { value: "steer", label: "steer — Record diagnostics and send findings to the agent" },
];
const SCOPES: readonly PolicyChoice<PolicySettings["scope"]>[] = [
  { value: "repository", label: "repository — All eligible files in this workspace" },
  { value: "touched-files", label: "touched-files — Only selected or changed files" },
  { value: "ai-output", label: "ai-output — Only captured AI-generated edits (automatic scans)" },
];
const EXPLICIT_SCOPES: readonly PolicyChoice<"repository" | "git-diff">[] = [
  { value: "repository", label: "repository — All eligible files in this workspace" },
  { value: "git-diff", label: "git-diff — Tracked and untracked files changed in Git" },
];
const SETTINGS_FIELDS: ReadonlyArray<{
  key: keyof PolicySettings;
  label: string;
  flag: string;
  choices: readonly PolicyChoice[];
}> = [
  { key: "scope", label: "Ongoing scope", flag: "--scope", choices: SCOPES },
  { key: "mode", label: "Ongoing mode", flag: "--mode", choices: MODES },
];

async function selectPolicyChoice<Value extends string>(
  ctx: RerailCommandContext,
  title: string,
  choices: readonly PolicyChoice<Value>[],
): Promise<Value | undefined> {
  const selected = await ctx.ui.select(title, choices.map(choice => choice.label));
  return choices.find(choice => choice.label === selected)?.value;
}

interface LauncherResult {
  code: number;
  stdout: string;
  stderr: string;
}

async function runLauncher(options: RerailCommandOptions, args: readonly string[]): Promise<LauncherResult> {
  const child = spawn(nodeExecutable(), [join(options.packageRoot, "dist", "cli.js"), ...args], {
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

async function runSettings(options: RerailCommandOptions, target: readonly string[], patch: readonly string[] = []): Promise<PolicyControls> {
  const result = await runLauncher(options, ["settings", ...target, ...patch, "--json"]);
  if (result.code !== 0) throw new Error((result.stdout + result.stderr).trim() || `Settings exited with code ${result.code}`);
  return policyControlsSchema.strip().parse(JSON.parse(result.stdout));
}

/** The notice for a page that was just opened; an Orca tab says so, the desktop browser needs no qualifier. */
function openingNotice(url: string, opening: PageOpening): string {
  return opening === "orca-tab" ? `Opening ${url} in the Orca window` : `Opening ${url}`;
}

export function registerRerailCommands(pi: RerailCommandHost, options: RerailCommandOptions): void {
  function register(name: string, description: string, action: (ctx: RerailCommandContext) => Promise<void>): void {
    pi.registerCommand(name, {
      description,
      async handler(args, ctx) {
        if (args.trim() !== "") {
          ctx.ui.notify(`/${name} takes no arguments. Run it on its own to use its controls.`, "error");
          return;
        }
        try {
          await action(ctx);
        } catch (error) {
          ctx.ui.notify(`/${name}: ${error instanceof Error ? error.message : String(error)}`, "error");
        }
      },
    });
  }

  /** The session's presentation: text for the status line and the page that shows it. */
  const sessionPresentation = async () => {
    const sessionId = options.sessionId?.();
    if (sessionId === undefined) throw new Error("No active session.");
    return readPresentation({ sessionId, host: options.host ?? "omp", cwd: options.cwd(), onDemand: true });
  };
  const unavailable = (error: unknown): Error => new Error(`Rerail unavailable: ${String(error instanceof Error ? error.message : error)
    .replace(/[\p{Cc}\p{Cf}\p{Zl}\p{Zp}]/gu, "")}`);

  register("rerail", "Show Rerail details for this session", async ctx => {
    let presentation;
    try {
      presentation = await sessionPresentation();
    } catch (error) {
      throw unavailable(error);
    }
    const link = presentation.url === null ? "" : `\n${presentation.url}`;
    ctx.ui.notify(`${presentation.text}${link}`, "info");
  });

  register("rerail-ui", "Open the Rerail rules page in the browser", async ctx => {
    const page = await readRulesPage({ cwd: options.cwd() });
    // The page needs a reachable daemon; the reason says why there is none.
    if (page.url === null) throw new Error(`no Rerail page to open: ${page.reason}`);
    ctx.ui.notify(openingNotice(page.url, await openPage(page.url)), "info");
  });

  register("rerail-session", "Open this session's Rerail page in the browser", async ctx => {
    let presentation;
    try {
      presentation = await sessionPresentation();
    } catch (error) {
      throw unavailable(error);
    }
    // The page needs a reachable daemon; the presentation text says why there is none.
    if (presentation.url === null) throw new Error(`no Rerail page to open: ${presentation.text}`);
    ctx.ui.notify(openingNotice(presentation.url, await openPage(presentation.url)), "info");
  });

  // Start and stop run the launcher, so the CLI owns the wording; without a terminal it addresses the coding agent's user.
  const repositoryCommands = [
    { name: "rerail-start", description: "Start Rerail in this checkout, setting the repo up the first time", argv: ["start"] },
    { name: "rerail-stop", description: "Stop Rerail in this checkout; rules and history are kept", argv: ["stop"] },
  ];
  for (const command of repositoryCommands) {
    register(command.name, command.description, async ctx => {
      const database = options.databasePath?.();
      const result = await runLauncher(options, database === undefined ? command.argv : [...command.argv, "--database", database]);
      // Re-attaching attaches the runtime where Rerail is started, and leaves only the "not on here" footer where it is not.
      if (result.code === 0) await options.reattach?.();
      ctx.ui.notify((result.stdout + result.stderr).trim() || `/${command.name} exited with code ${result.code}.`, result.code === 0 ? "info" : "error");
    });
  }

  for (const command of LAUNCHER_COMMANDS) {
    register(command.name, command.description, async ctx => {
      const resetting = command.name === "rerail-reset";
      if (resetting && !await ctx.ui.confirm("Reset Rerail history?", "Delete this repository's Rerail session history, across all its worktrees? Rules are kept. This cannot be undone.")) return;
      let result: LauncherResult;
      try {
        if (resetting) await options.detach?.();
        result = await runLauncher(options, command.argv);
      } finally {
        if (resetting) await options.reattach?.();
      }
      ctx.ui.notify((result.stdout + result.stderr).trim() || `/${command.name} exited with code ${result.code}.`, result.code === 0 ? "info" : "error");
    });
  }

  const controls = [
    { name: "rerail-settings", description: "View and change ongoing scan scope and mode" },
    { name: "rerail-scan", description: "Run a one-shot scan: choose scope, mode, and file paths" },
  ];
  for (const command of controls) {
    register(command.name, command.description, async ctx => {
      if (ctx.hasUI === false) throw new Error("Interactive controls require a terminal UI. Use the rerail CLI outside the harness.");
      const sessionId = options.sessionId?.();
      if (sessionId === undefined) {
        throw new Error("Scan controls are unavailable before this host has an active session.");
      }
      // Keep the dialog bound to the session and checkout in which it was opened.
      const cwd = options.cwd();
      const launcher = { ...options, cwd: () => cwd };
      const target = ["--session", sessionId];
      let current = await runSettings(launcher, target);
      if (command.name === "rerail-settings") {
        for (;;) {
          const labels = SETTINGS_FIELDS.map(field => `${field.label}: ${current.settings[field.key]}`);
          const selected = await ctx.ui.select(`Rerail settings — ${current.workspaceRoot}\nChanges save immediately; running scans keep their settings.`, [...labels, "Done"]);
          const field = SETTINGS_FIELDS.find((_field, index) => labels[index] === selected);
          if (field === undefined) return;
          const value = await selectPolicyChoice(ctx, `${field.label} (current: ${current.settings[field.key]})`, field.choices);
          if (value === undefined) continue;
          current = await runSettings(launcher, target, [field.flag, value]);
          ctx.ui.notify(`Saved ${field.label.toLowerCase()}: ${current.settings[field.key]}`, "info");
        }
      }

      const scope = await selectPolicyChoice(ctx, "One-shot diagnostic scope", EXPLICIT_SCOPES);
      if (scope === undefined) return;
      const argv = ["scan", ...target, "--scope", scope, "--wait"];
      ctx.ui.notify(`Scanning ${scope} in log-only mode; saved defaults are unchanged.`, "info");
      const result = await runLauncher(launcher, argv);
      ctx.ui.notify((result.stdout + result.stderr).trim() || `Scan exited with code ${result.code}.`, result.code === 0 || result.code === 2 ? "info" : "error");
    });
  }
}
