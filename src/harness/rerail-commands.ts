/**
 * The Rerail commands every coding agent gets, and the instructions each one carries. Claude Code and
 * Cursor read them as slash-command files, Codex as skills (`$rerail-ui`), and OMP and Pi register them
 * in-process (`host-commands.ts`). One catalog keeps the names, descriptions and `/rerail` list the same
 * everywhere.
 */

export type RerailCommandName =
  | "rerail"
  | "rerail-ui"
  | "rerail-create"
  | "rerail-stop"
  | "rerail-start"
  | "rerail-update"
  | "rerail-rage-quit";

export interface RerailCommand {
  name: RerailCommandName;
  /** One line for the coding agent's command menu. */
  description: string;
  /** The `/rerail` list entry: lower case, a few words. */
  summary: string;
}

export const RERAIL_COMMANDS: readonly RerailCommand[] = [
  { name: "rerail", description: "What Rerail is doing in this repo, and every Rerail command", summary: "what Rerail is doing here" },
  { name: "rerail-ui", description: "Open the Rerail dashboard", summary: "open the dashboard" },
  { name: "rerail-create", description: "Create a rule for this repo, in your own words", summary: "create a rule, in your own words" },
  { name: "rerail-stop", description: "Stop Rerail in this checkout and the worktrees inside it; rules and history are kept", summary: "stop Rerail in this repo" },
  { name: "rerail-start", description: "Start Rerail in this checkout and the worktrees inside it, setting the repo up the first time", summary: "start Rerail in this repo" },
  { name: "rerail-update", description: "Update Rerail to the newest version", summary: "get the newest version" },
  { name: "rerail-rage-quit", description: "Remove Rerail from this computer; rules and history are kept in a backup", summary: "remove Rerail from this computer" },
];

/** How a coding agent invokes a command: Codex runs skills as `$name`, every other agent as `/name`. */
export type CommandSigil = "/" | "$";

/** The `Commands` block `/rerail` ends with: every other command, one per line, with what it does. */
export function rerailCommandList(sigil: CommandSigil): string {
  const entries = RERAIL_COMMANDS.filter(command => command.name !== "rerail");
  const width = Math.max(...entries.map(command => command.name.length)) + 1 + 3;
  return [
    "Commands",
    ...entries.map(command => (sigil + command.name).padEnd(width) + command.summary),
    "Or just ask me, in plain words.",
  ].join("\n");
}

export interface CreateRuleInstructionsInput {
  /** The shell-quoted launcher the agent runs, e.g. `'/usr/bin/node' '/home/me/.rerail/current/dist/cli/cli.js'`. */
  cli: string;
  /** Where the installed package keeps the rerail-create skill (`skills/rerail-create/`). */
  packageRoot: string;
  /** The person's sentence; a placeholder such as `$ARGUMENTS` when the coding agent substitutes it. */
  request: string;
}

/** The skill `/rerail-create` follows, inside the installed package. */
export function createRuleSkillPath(packageRoot: string): string {
  return `${packageRoot}/skills/rerail-create/SKILL.md`;
}

/**
 * `/rerail-create`: suggest rules from this repo's sessions and standards, or write the one the person names. The skill in
 * the package holds the steps; this keeps the commands and the one promise that matters in the command itself: test before
 * showing, and add only after the person says yes.
 */
export function createRuleInstructions({ cli, packageRoot, request }: CreateRuleInstructionsInput): string {
  return [
    `Create a Rerail rule for this repo. The person's words: ${request}`,
    "",
    `Read ${createRuleSkillPath(packageRoot)} and follow it. Read its references when a step names one.`,
    `Wherever it says \`rerail\`, run \`${cli}\`.`,
    "",
    "- With words: write that rule (the skill's Write).",
    "- With no words: suggest rules from this repo's sessions and standards (the skill's Suggest), then write the ones the person picks.",
    "- If the words say a rule is wrong or noisy: tune it (the skill's Tune).",
    "",
    "The commands it uses:",
    "",
    "```sh",
    `${cli} rules evidence --json`,
    `${cli} rules export --output <tmp>/rules.json`,
    `${cli} rules test <draft.json> --json`,
    `${cli} rules add <draft.json>`,
    "```",
    "",
    "Test every draft before showing it, read it back with \"Add this rule?\", and run `rules add` only after the person says yes.",
    "Never add a rule the person has not confirmed. The rule belongs to this repo only: never offer \"every repo\". Never write instruction files.",
  ].join("\n");
}

/** `/rerail-rage-quit`: say what goes and what stays, ask once, and remove only on an explicit yes (design A9, A10). */
export function rageQuitInstructions(cli: string, sigil: CommandSigil): string {
  return [
    "Remove Rerail from this computer, if the person confirms.",
    "",
    "First see what would be removed, without changing anything:",
    "",
    "```sh",
    `${cli} install --remove --dry-run`,
    "```",
    "",
    "Then show this, naming the coding agents from that output, and ask once:",
    "",
    "```text",
    "This removes Rerail from this computer.",
    "  Removes   Rerail, from <coding agents>, in every repo",
    "  Keeps     your rules and history, in a backup folder",
    "",
    "Remove Rerail from this computer?",
    "You can install it again later and get your rules back.",
    `  1. Yes, remove Rerail`,
    `  2. Just stop it in this repo (${sigil}rerail-stop)`,
    "  3. Cancel",
    "```",
    "",
    "Only if the person picks 1, run:",
    "",
    "```sh",
    `${cli} install --remove`,
    "```",
    "",
    "Then reply with this, using the backup folder from the output:",
    "",
    "```text",
    "○ Rerail is removed from this computer.",
    "  Removed from   <coding agents>",
    "  Backup         <backup folder>   your rules and history",
    "  To come back   install Rerail again, and ask it to restore the backup",
    "```",
    "",
    `If the person picks 2, run \`${cli} stop\` and report its output. If they pick 3 or anything else, change nothing.`,
    "Never run the removal without the person's explicit yes in this session.",
  ].join("\n");
}
