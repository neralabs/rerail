import {
  createRuleInstructions,
  uninstallInstructions,
  showMeInstructions,
  RERAIL_COMMANDS,
  type CommandSigil,
  type RerailCommand,
  type RerailCommandName,
} from "./rerail-commands.js";
import { SETUP_FLAGS } from "./setup-screens.js";

/** Coding agents that read Rerail commands from files: slash commands (Claude Code, Cursor) or skills (Codex). */
export type CommandFileAgent = "claude-code" | "cursor" | "codex";

export interface CommandFileContext {
  /** The shell-quoted launcher, `<node> <root>/dist/cli/cli.js`, as the hooks run it. */
  cli: string;
  /** The installed package root the launcher belongs to. */
  packageRoot: string;
  /**
   * The stable launcher (`~/.rerail/bin/rerail`), quoted as set-up quotes it: set-up's answer commands start with
   * it, or with `rerail setup` until it exists.
   */
  setupLauncher: string;
}

/** Marks a Codex skill as installed by Rerail, so registration only ever replaces or removes its own. */
export const MANAGED_SKILL_MARKER = "<!-- Managed by @neralabs/rerail -->";

interface CommandBody {
  text: string;
  /** Exact launcher invocations a Claude Code command may run without asking; `*` ends an open argument list. */
  allowed: readonly string[];
}

const REPORT_VERBATIM = "Do not run anything else. If the command fails, show the error and stop.";

/** Codex runs the launcher only through its own approval boundary; a sandbox cannot reach the loopback daemon. */
const CODEX_APPROVAL = [
  "Run the command through Codex's normal local-execution approval (`sandbox_permissions: \"require_escalated\"`),",
  "with a one-line reason that it reads or changes Rerail on this computer. If approval is denied, say so; never disable",
  "the sandbox, change its configuration, or add bypass flags.",
].join("\n");

function runOne(command: string, lead: string): string {
  return [lead, "", "Run exactly this and report its output verbatim:", "", "```sh", command, "```", "", REPORT_VERBATIM].join("\n");
}

/**
 * `/rerail-start`: where the repo is not set up, `start` prints set-up's next question; the agent relays it and runs the
 * answer's command. `--relay` keeps set-up in the agent lane even where the coding agent runs commands in a terminal, and
 * every answer command set-up prints carries it.
 */
function startBody(cli: string, setupLauncher: string): CommandBody {
  const start = `${cli} start ${SETUP_FLAGS.relay}`;
  return {
    text: [
      "Start Rerail in this checkout and the worktrees inside it, setting the repo up the first time.",
      "",
      "Run exactly this:",
      "",
      "```sh",
      start,
      "```",
      "",
      "If the output asks a set-up question, relay it to the person as written and wait for their reply. Do not choose for them.",
      "When they answer, run only the one command the output gives for that answer, exactly as printed, then do the same with",
      "its output. Repeat until set-up stops asking, then report the last output verbatim.",
      "",
      "Do not run anything else. If a command fails, show the error and stop.",
    ].join("\n"),
    allowed: [start, `${setupLauncher} setup ${SETUP_FLAGS.relay} *`, `rerail setup ${SETUP_FLAGS.relay} *`],
  };
}

/**
 * How each agent's command names its session to the launcher, as `rerail session` and `rerail stop` take it. Claude Code
 * substitutes `${CLAUDE_SESSION_ID}` in command files (the quotes keep it one shell word); the launcher reads Codex's
 * thread from `CODEX_THREAD_ID`; Cursor names only itself.
 */
const SESSION_ARGUMENTS: Record<CommandFileAgent, string> = {
  "claude-code": " claude-code '${CLAUDE_SESSION_ID}'",
  cursor: " cursor",
  codex: "",
};

const CODEX_IDENTITY = "\n\nThe launcher reads this session's identity from `CODEX_THREAD_ID`; never pick another or the newest session.";

/** `/rerail` and its alias `/rerail-help`: the L1 summary for this session (or this repo, where the agent cannot name its session). */
function overviewBody(agent: CommandFileAgent, cli: string): CommandBody {
  const command = `${cli} session${SESSION_ARGUMENTS[agent]}`;
  return {
    text: `${runOne(command, "Show what Rerail is doing here, and every Rerail command.")}${agent === "codex" ? CODEX_IDENTITY : ""}`,
    allowed: [`${cli} session claude-code *`],
  };
}

/**
 * `/rerail-stop`: the coding agent goes along so the reply uses its command sigil, with its session where it can name
 * one, so the reply names the rules that steered it (design N9). Codex names its host and the launcher reads the thread
 * from `CODEX_THREAD_ID`; Cursor cannot name its session, and its sigil is the default, so it stops without either.
 */
function stopBody(agent: CommandFileAgent, cli: string): CommandBody {
  const command = `${cli} stop${agent === "cursor" ? "" : agent === "codex" ? " codex" : SESSION_ARGUMENTS[agent]}`;
  const lead = "Stop Rerail in this checkout and the worktrees inside it. Rules and history are kept.";
  return {
    text: `${runOne(command, lead)}${agent === "codex" ? CODEX_IDENTITY : ""}`,
    allowed: [`${cli} stop claude-code *`],
  };
}

/** `/rerail-manage`: read the URL first, so opening is the only step that launches anything. */
function dashboardBody(agent: CommandFileAgent, cli: string): CommandBody {
  if (agent === "codex") return { text: runOne(`${cli} manage`, "Open the Rerail dashboard for this repo in the browser."), allowed: [] };
  return {
    text: [
      "Open the Rerail dashboard for this repo.",
      "",
      "Get the URL without opening anything. Run exactly this and read the URL from its output:",
      "",
      "```sh",
      `${cli} manage --print`,
      "```",
      "",
      "If the command fails, show the error and stop.",
      "",
      "Open the URL as a tab in this window before reaching for the desktop browser, in this order:",
      "",
      ...(agent === "claude-code"
        ? [`- Under Orca (\`ORCA_TAB_ID\` set): run \`${cli} manage\`. It opens the page as a tab in the Orca window and falls back to the desktop browser on its own.`]
        : []),
      "- In Cursor with a built-in browser tool: navigate that browser to the URL; it opens as a tab in the Cursor window.",
      `- Otherwise let the launcher open the desktop browser: \`${cli} manage\`.`,
      "",
      "Report the URL and where it was opened. Do not run anything else.",
    ].join("\n"),
    allowed: [`${cli} manage --print`, `${cli} manage`],
  };
}

type BodyBuilder = (agent: CommandFileAgent, context: CommandFileContext) => CommandBody;

const sigilFor = (agent: CommandFileAgent): CommandSigil => agent === "codex" ? "$" : "/";

function createBody(agent: CommandFileAgent, { cli, packageRoot }: CommandFileContext): CommandBody {
  const request = agent === "claude-code" ? "\"$ARGUMENTS\""
    : `the words the person typed after ${sigilFor(agent)}rerail-create`;
  return {
    text: createRuleInstructions({ cli, packageRoot, request }),
    // `rules export --output` writes any path, so Claude Code asks before it runs; the others only read or add a rule.
    allowed: [`${cli} rules evidence *`, `${cli} rules test *`, `${cli} rules add *`],
  };
}

/** Every command's body; the `Record` makes a command without one a type error. */
const COMMAND_BODIES: Record<RerailCommandName, BodyBuilder> = {
  "rerail": (agent, { cli }) => overviewBody(agent, cli),
  "rerail-help": (agent, { cli }) => overviewBody(agent, cli),
  "rerail-manage": (agent, { cli }) => dashboardBody(agent, cli),
  "rerail-create": createBody,
  "rerail-start": (_agent, { cli, setupLauncher }) => startBody(cli, setupLauncher),
  "rerail-stop": (agent, { cli }) => stopBody(agent, cli),
  "rerail-update": (_agent, { cli }) => ({
    text: runOne(`${cli} update`, "Update Rerail to the newest version.") + "\n\nAfterwards, tell the person that new sessions use the new version.",
    allowed: [`${cli} update`],
  }),
  // The removal itself is left out of `allowed`, so Claude Code asks the person once more before running it.
  "rerail-uninstall": (agent, { cli }) => ({
    text: uninstallInstructions(cli, sigilFor(agent)),
    allowed: [`${cli} install --remove --dry-run`, `${cli} stop`],
  }),
  // The demo file's write and fix still go through the coding agent's own edit permission.
  "rerail-show-me": (agent, { cli }) => ({
    text: showMeInstructions({ cli, sigil: sigilFor(agent), write: agent === "codex" ? "shell" : "edit-tool" }),
    allowed: [`${cli} show-me --json`, `${cli} show-me --cleanup`],
  }),
};

function command(name: RerailCommandName): RerailCommand {
  const found = RERAIL_COMMANDS.find(entry => entry.name === name);
  if (found === undefined) throw new Error(`Unknown Rerail command: ${name}`);
  return found;
}

/**
 * The file a coding agent reads for `name`. Claude Code takes front matter (`description`, `allowed-tools`);
 * Cursor shows the first line as the description; Codex skills need `name` and `description` front matter.
 */
export function commandFileText(agent: CommandFileAgent, name: RerailCommandName, context: CommandFileContext): string {
  const { description } = command(name);
  const body = COMMAND_BODIES[name](agent, context);
  if (agent === "cursor") return `${description}.\n\n${body.text}\n`;
  if (agent === "codex") {
    const text = `${body.text}\n\n${CODEX_APPROVAL}`;
    return ["---", `name: ${name}`, `description: ${description}.`, "---", "", text, "", MANAGED_SKILL_MARKER, ""].join("\n");
  }
  const allowed = body.allowed.map(invocation => `Bash(${invocation})`).join(", ");
  return [
    "---",
    `description: ${description}`,
    ...(allowed === "" ? [] : [`allowed-tools: ${allowed}`]),
    "---",
    "",
    body.text,
    "",
  ].join("\n");
}
