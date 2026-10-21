/**
 * The Rerail commands every coding agent gets, and the instructions each one carries. Claude Code and
 * Cursor read them as slash-command files, Codex as skills (`$rerail-manage`), and OMP and Pi register them
 * in-process (`host-commands.ts`). One catalog keeps the names, descriptions and `/rerail` list the same
 * everywhere.
 */

export type RerailCommandName =
  | "rerail"
  | "rerail-manage"
  | "rerail-create"
  | "rerail-stop"
  | "rerail-start"
  | "rerail-update"
  | "rerail-uninstall"
  | "rerail-help"
  | "rerail-show-me";

export interface RerailCommand {
  name: RerailCommandName;
  /** One line for the coding agent's command menu. */
  description: string;
  /** The `/rerail` list entry: lower case, a few words. */
  summary: string;
}

export const RERAIL_COMMANDS: readonly RerailCommand[] = [
  { name: "rerail", description: "What Rerail is doing in this repo, and every Rerail command", summary: "what Rerail is doing here" },
  { name: "rerail-manage", description: "Open the Rerail dashboard", summary: "open the dashboard" },
  { name: "rerail-create", description: "Create a rule for this repo, in your own words", summary: "create a rule, in your own words" },
  { name: "rerail-stop", description: "Stop Rerail in this checkout and the worktrees inside it; rules and history are kept", summary: "stop Rerail in this repo" },
  { name: "rerail-start", description: "Start Rerail in this checkout and the worktrees inside it, setting the repo up the first time", summary: "start Rerail in this repo" },
  { name: "rerail-update", description: "Update Rerail to the newest version", summary: "get the newest version" },
  { name: "rerail-uninstall", description: "Remove Rerail from this computer; rules and history are kept in a backup", summary: "remove Rerail from this computer" },
  // An alias of `/rerail`: the same summary and command list.
  { name: "rerail-help", description: "Show a list of Rerail commands", summary: "show a list of commands" },
  { name: "rerail-show-me", description: "See Rerail catch a broken rule in this repo and steer the fix, then clean up", summary: "see Rerail catch and fix a broken rule" },
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

/** `/rerail-uninstall`: say what goes and what stays, ask once, and remove only on an explicit yes (design A9, A10). */
export function uninstallInstructions(cli: string, sigil: CommandSigil): string {
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

export interface ShowMeInstructionsInput {
  /** The shell-quoted launcher the agent runs. */
  cli: string;
  sigil: CommandSigil;
  /**
   * How the agent writes the demo file. Codex's hooks see a write made through its shell tool, not through
   * apply_patch, so Codex writes with one shell command; every other agent uses its own edit tool.
   */
  write: "edit-tool" | "shell";
}

/**
 * `/rerail-show-me`: `rerail show-me` adds a demo rule scoped to one demo file; the agent breaks it on purpose, reads
 * the file straight after so the steer reaches it (Claude Code and Cursor deliver it with the next tool call), fixes
 * it, and always runs `rerail show-me --cleanup`. The steer is recognised by the rule id, never faked.
 */
export function showMeInstructions({ cli, sigil, write }: ShowMeInstructionsInput): string {
  const how = write === "shell"
    ? "with one shell command, not a file-change, edit, write or apply_patch tool (for example `cat > '<file>' <<'EOF'` … `EOF`)"
    : "with your file edit or write tool";
  return [
    "Show the person how Rerail works, on this repo, in under a minute: add a demo rule, break it on purpose, let Rerail",
    "steer you to the fix, then clean up.",
    "",
    "1. Run exactly this and read its JSON output:",
    "",
    "```sh",
    `${cli} show-me --json`,
    "```",
    "",
    `   If it fails, show its message and stop; where Rerail is not on or the repo is not set up, tell the person to type ${sigil}rerail-start first.`,
    "   The output names the demo rule (`ruleId`), the demo file (`file`, and `path` in this repo), what to write first",
    "   (`violating`), the fix (`fixed`) and the rule's `message`.",
    "",
    "2. Tell the person, in one line: \"I'll add a demo rule, break it on purpose, and let Rerail steer me. Then I'll clean up.\"",
    "",
    `3. Create \`file\` containing exactly the \`violating\` text, ${how}. Touch no other file.`,
    "",
    "4. Do not end your turn or say that you are waiting. Straight after the write, make a separate read-only tool call that",
    "   reads `file`: Rerail's steer comes with the write's result or with that read, and names the rule `ruleId`.",
    "   If it has not come by then, read `file` once more.",
    "",
    "5. When the steer arrives, fix `file` as its message says, the same way you wrote it: its content becomes exactly the",
    "   `fixed` text. If no steer arrived by the end of the second read, fix nothing and never pretend one came.",
    "",
    "6. Always clean up, whatever happened. Run exactly this:",
    "",
    "```sh",
    `${cli} show-me --cleanup`,
    "```",
    "",
    "7. Reply with this, filled in:",
    "",
    "```text",
    "● That was Rerail.",
    "  The rule     <message>",
    "  Broken       <path>, on purpose",
    "  Steered      Rerail told me as I worked, and I fixed it. I wasn't stopped.",
    "  Cleaned up   the demo file and the demo rule are gone",
    `  Your turn    ${sigil}rerail-create  create a rule, in your own words`,
    "```",
    "",
    `   Without a steer, the Steered row reads "No steer arrived, so nothing was fixed. Type ${sigil}rerail to see what Rerail is doing here."`,
    "   If the cleanup failed, the Cleaned up row shows its error instead.",
    "",
    "Never edit any file but the demo file. Never write instruction files.",
  ].join("\n");
}
