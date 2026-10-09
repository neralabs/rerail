import {
  createRuleInstructions,
  rageQuitInstructions,
  RERAIL_COMMANDS,
  type RerailCommand,
  type RerailCommandName,
} from "./rerail-commands.js";

/** Coding agents that read Rerail commands from files: slash commands (Claude Code, Cursor) or skills (Codex). */
export type CommandFileAgent = "claude-code" | "cursor" | "codex";

export interface CommandFileContext {
  /** The shell-quoted launcher, `<node> <root>/dist/cli/cli.js`, as the hooks run it. */
  cli: string;
  /** The installed package root the launcher belongs to. */
  packageRoot: string;
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

/** `/rerail`: the L1 summary for this session (or this repo, where the agent cannot name its session). */
function overviewBody(agent: CommandFileAgent, cli: string): CommandBody {
  const sessionArguments: Record<CommandFileAgent, string> = {
    // Claude Code substitutes `${CLAUDE_SESSION_ID}` in command files; the quotes keep it one shell word.
    "claude-code": " claude-code '${CLAUDE_SESSION_ID}'",
    cursor: " cursor",
    codex: "",
  };
  const command = `${cli} session${sessionArguments[agent]}`;
  const identity = agent === "codex"
    ? "\n\nThe launcher reads this session's identity from `CODEX_THREAD_ID`; never pick another or the newest session."
    : "";
  return {
    text: `${runOne(command, "Show what Rerail is doing here, and every Rerail command.")}${identity}`,
    allowed: [`${cli} session claude-code *`],
  };
}

/** `/rerail-ui`: read the URL first, so opening is the only step that launches anything. */
function dashboardBody(agent: CommandFileAgent, cli: string): CommandBody {
  if (agent === "codex") return { text: runOne(`${cli} ui`, "Open the Rerail dashboard for this repo in the browser."), allowed: [] };
  return {
    text: [
      "Open the Rerail dashboard for this repo.",
      "",
      "Get the URL without opening anything. Run exactly this and read the URL from its output:",
      "",
      "```sh",
      `${cli} ui --print`,
      "```",
      "",
      "If the command fails, show the error and stop.",
      "",
      "Open the URL as a tab in this window before reaching for the desktop browser, in this order:",
      "",
      ...(agent === "claude-code"
        ? [`- Under Orca (\`ORCA_TAB_ID\` set): run \`${cli} ui\`. It opens the page as a tab in the Orca window and falls back to the desktop browser on its own.`]
        : []),
      "- In Cursor with a built-in browser tool: navigate that browser to the URL; it opens as a tab in the Cursor window.",
      `- Otherwise let the launcher open the desktop browser: \`${cli} ui\`.`,
      "",
      "Report the URL and where it was opened. Do not run anything else.",
    ].join("\n"),
    allowed: [`${cli} ui --print`, `${cli} ui`],
  };
}

function commandBody(agent: CommandFileAgent, name: RerailCommandName, context: CommandFileContext): CommandBody {
  const { cli, packageRoot } = context;
  const sigil = agent === "codex" ? "$" : "/";
  switch (name) {
    case "rerail":
      return overviewBody(agent, cli);
    case "rerail-ui":
      return dashboardBody(agent, cli);
    case "rerail-create": {
      const request = agent === "claude-code" ? "\"$ARGUMENTS\""
        : `the words the person typed after ${sigil}rerail-create`;
      return {
        text: createRuleInstructions({ cli, packageRoot, request }),
        allowed: [`${cli} rules test *`, `${cli} rules add *`],
      };
    }
    case "rerail-start":
      return {
        text: runOne(`${cli} start`, "Start Rerail in this checkout and the worktrees inside it, setting the repo up the first time with its recommended rule packs."),
        allowed: [`${cli} start`],
      };
    case "rerail-stop":
      return {
        text: runOne(`${cli} stop`, "Stop Rerail in this checkout and the worktrees inside it. Rules and history are kept."),
        allowed: [`${cli} stop`],
      };
    case "rerail-update":
      return {
        text: runOne(`${cli} update`, "Update Rerail to the newest version.") + "\n\nAfterwards, tell the person that new sessions use the new version.",
        allowed: [`${cli} update`],
      };
    case "rerail-rage-quit":
      // The removal itself is left out of `allowed`, so Claude Code asks the person once more before running it.
      return { text: rageQuitInstructions(cli, sigil), allowed: [`${cli} install --remove --dry-run`, `${cli} stop`] };
  }
}

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
  const body = commandBody(agent, name, context);
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
