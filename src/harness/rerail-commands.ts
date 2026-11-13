/**
 * The Rerail commands every coding agent gets, and the instructions each one carries. Claude Code and
 * Cursor read them as slash-command files, Codex as skills (`$rerail-manage`), and OMP and Pi register them
 * in-process (`host-commands.ts`). One catalog keeps the names, descriptions and `/rerail` list the same
 * everywhere.
 */

import type { CommandSigil, ObservedFileWrite } from "../engine/adapter-contract.js";
import { powershellQuotesTwice, type Shell } from "../engine/command-line.js";

export type RerailCommandName =
  | "rerail"
  | "rerail-manage"
  | "rerail-create"
  | "rerail-stop"
  | "rerail-start"
  | "rerail-update"
  | "rerail-uninstall"
  | "rerail-help"
  | "rerail-show-me"
  | "rerail-bug-report";

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
  { name: "rerail-create", description: "Write a rule in your own words, get rule suggestions for this repo, or change one", summary: "write, suggest or change a rule" },
  { name: "rerail-stop", description: "Stop Rerail in this checkout and the worktrees inside it; rules and history are kept", summary: "stop Rerail in this repo" },
  { name: "rerail-start", description: "Start Rerail in this checkout and the worktrees inside it, setting the repo up the first time", summary: "start Rerail in this repo" },
  { name: "rerail-update", description: "Update Rerail to the newest version", summary: "get the newest version" },
  { name: "rerail-uninstall", description: "Remove Rerail from this computer, keeping your rules and history in a backup or deleting everything", summary: "remove Rerail from this computer" },
  // An alias of `/rerail`: the same summary and command list.
  { name: "rerail-help", description: "Show a list of Rerail commands", summary: "show a list of commands" },
  { name: "rerail-show-me", description: "See Rerail catch a broken rule in this repo and steer the fix, then clean up", summary: "see Rerail catch and fix a broken rule" },
  { name: "rerail-bug-report", description: "Work out what went wrong with Rerail, then fix your own rule or report a Rerail bug in an issue", summary: "diagnose a bug, then fix or report it" },
];

/** The `rerail` commands a terminal answers, named at the end of `/rerail` so no one goes looking for them. */
export const TERMINAL_COMMANDS_LINE = "In a terminal: rerail status · rerail version · rerail update · rerail daemon stop";

/**
 * The `Commands` block `/rerail` ends with: every other command, one per line, with what it does, then the terminal
 * commands, since the coding agent's list is where people look for a `rerail` command first.
 */
export function rerailCommandList(sigil: CommandSigil): string {
  const entries = RERAIL_COMMANDS.filter(command => command.name !== "rerail");
  const width = Math.max(...entries.map(command => command.name.length)) + 1 + 3;
  return [
    "Commands",
    ...entries.map(command => (sigil + command.name).padEnd(width) + command.summary),
    "Or just ask me, in plain words.",
    "",
    TERMINAL_COMMANDS_LINE,
  ].join("\n");
}

/**
 * The fence a command for `shell` goes in. A PowerShell line (`& '<node>' …`) in a `sh` fence invites an agent with a
 * POSIX shell at hand to run it there, where the `&` puts node in the background with no arguments.
 */
const FENCES: Record<Shell, string> = { posix: "```sh", powershell: "```powershell" };

export function commandFence(shell: Shell = "posix"): string {
  return FENCES[shell];
}

export interface CreateRuleInstructionsInput {
  /** The shell-quoted launcher the agent runs, e.g. `'/usr/bin/node' '/home/me/.rerail/current/dist/cli/cli.js'`. */
  cli: string;
  /** Where the installed package keeps the rerail-create skill (`skills/rerail-create/`). */
  packageRoot: string;
  /** The person's sentence; a placeholder such as `$ARGUMENTS` when the coding agent substitutes it. */
  request: string;
  /** The shell `cli` is written for, which names the fence its commands go in. */
  shell?: Shell;
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
export function createRuleInstructions({ cli, packageRoot, request, shell }: CreateRuleInstructionsInput): string {
  return [
    `Create a Rerail rule for this repo. The person's words: ${request}`,
    "",
    `Read ${createRuleSkillPath(packageRoot)} and follow it. Read its references when a step names one.`,
    `Wherever it says \`rerail\`, run \`${cli}\`.`,
    "",
    "- With words: write that rule (the skill's Write).",
    "- With no words: suggest rules from this repo's sessions and standards (the skill's Suggest), then write the ones the person picks.",
    "- If the words say a rule is wrong or noisy: tune it (the skill's Tune).",
    "- If the words change only where a rule applies: change its scope and nothing else (the skill's Scope).",
    "",
    "The commands it uses:",
    "",
    commandFence(shell),
    `${cli} rules evidence --json`,
    `${cli} rules export --output <tmp>/rules.json`,
    `${cli} rules show <id>`,
    `${cli} rules test <draft.json> --json`,
    `${cli} rules add <draft.json>`,
    `${cli} rules approve`,
    `${cli} rules approve <id>`,
    "```",
    "",
    "Test every draft before showing it, read it back with \"Add this rule?\", and run `rules add` only after the person says yes.",
    "An added rule runs in Log, steering no one, until the person approves it. Audit any rule you did not write before asking (the skill's Approve), and run `rules approve <id>` only after they say yes to it.",
    "Never add a rule the person has not confirmed. The rule belongs to this repo only: never offer \"every repo\". Never write instruction files.",
  ].join("\n");
}

/**
 * `/rerail-uninstall`: say what goes and what stays, offer both removals (keeping a backup, or deleting everything with
 * `--delete-data`), ask once, and remove only on an explicit yes (design A9, A10).
 */
export function uninstallInstructions(cli: string, sigil: CommandSigil, shell: Shell = "posix"): string {
  return [
    "Remove Rerail from this computer, if the person confirms.",
    "",
    "First, see what each kind of removal would do, without changing anything:",
    "",
    commandFence(shell),
    `${cli} install --remove --dry-run`,
    `${cli} install --remove --delete-data --dry-run`,
    "```",
    "",
    "From the first output, take the coding agents and the backup folder. From the second, take every path on a",
    "`Would delete` line. Show this, as written, and ask once:",
    "",
    "```text",
    "Remove Rerail from this computer?",
    "",
    `  Removes   ${sigil}rerail commands, hooks and the status line`,
    "            from <coding agents>",
    "            Rerail itself: its installed versions, the rerail",
    "            command, rule packs and cache",
    "            Rerail stops in every repo",
    "",
    "  1. Remove, keep a backup        your rules and history move to",
    "                                  <backup folder>; reinstall and",
    "                                  restore them anytime",
    "  2. Remove and delete everything cannot be undone; also deletes:",
    "                                  <each Would delete path>",
    "  3. Only stop it in this repo    Rerail stays everywhere else",
    "  4. Cancel                       nothing changes",
    "```",
    "",
    "Under it, show each `Would keep` line of the second output and its line about checkouts Rerail never recorded, as",
    "written: those are what option 2 cannot reach, each with the command that deletes it.",
    "",
    "If they pick 1, run:",
    "",
    commandFence(shell),
    `${cli} install --remove`,
    "```",
    "",
    "Then reply with this, using the backup folder from the output:",
    "",
    "```text",
    "○ Rerail is removed from this computer.",
    "  Removed from   <coding agents>",
    "  Backup         <backup folder>",
    "",
    "  Restart your coding agents to finish.",
    "  To come back, install Rerail again and ask it to restore the backup.",
    "```",
    "",
    "If they pick 2, run:",
    "",
    commandFence(shell),
    `${cli} install --remove --delete-data`,
    "```",
    "",
    "Then reply with this:",
    "",
    "```text",
    "○ Rerail and everything it kept are deleted from this computer.",
    "  Removed from   <coding agents>",
    "",
    "  Restart your coding agents to finish.",
    "```",
    "",
    "Under it, show each `Kept` line of the output and its line about checkouts Rerail never recorded, as written.",
    "",
    "If the removal's output has `Could not remove` or `Could not delete` lines, show each as written: it names what is left and the command that removes it.",
    "",
    `If they pick 3, run \`${cli} stop\` and show its output.`,
    "If they pick 4, or anything else, change nothing.",
    "Never run either removal without the person's explicit yes in this session.",
  ].join("\n");
}

export interface ShowMeInstructionsInput {
  /** The shell-quoted launcher the agent runs. */
  cli: string;
  sigil: CommandSigil;
  /** How the agent writes the demo file so the hooks see it: its capability profile's `observedFileWrite`. */
  write: ObservedFileWrite;
  /** The shell the agent runs commands in: PowerShell for Codex on Windows (`windowsCommandShell`), which has no heredoc. */
  shell?: Shell;
}

/**
 * `/rerail-show-me`: `rerail show-me` adds a demo rule scoped to one demo file; the agent breaks it on purpose, reads
 * the file straight after so the steer reaches it (Claude Code and Cursor deliver it with the next tool call), fixes
 * it, and always runs `rerail show-me --cleanup`. The steer is recognised by the rule id, never faked.
 */
export function showMeInstructions({ cli, sigil, write, shell = "posix" }: ShowMeInstructionsInput): string {
  const example = shell === "powershell"
    ? `\`Set-Content -NoNewline -LiteralPath '<file>' -Value @'\` … \`'@\`, with ${powershellQuotesTwice("`<file>`")}`
    : "`cat > '<file>' <<'EOF'` … `EOF`";
  const how = write === "shell"
    ? `with one shell command, not a file-change, edit, write or apply_patch tool (for example ${example})`
    : "with your file edit or write tool";
  return [
    "Show the person how Rerail works, on this repo, in under a minute: add a demo rule, break it on purpose, let Rerail",
    "steer you to the fix, then clean up.",
    "",
    "1. Run exactly this and read its JSON output:",
    "",
    commandFence(shell),
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
    commandFence(shell),
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

export interface BugReportInstructionsInput {
  /** The shell-quoted launcher the agent runs, e.g. `'/usr/bin/node' '/home/me/.rerail/current/dist/cli/cli.js'`. */
  cli: string;
  /** Where the installed package keeps the rerail-bug-report skill (`skills/rerail-bug-report/`). */
  packageRoot: string;
  /** The person's words; a placeholder such as `$ARGUMENTS` when the coding agent substitutes it. */
  request: string;
  /** The shell `cli` is written for, which names the fence its commands go in. */
  shell?: Shell;
}

/** The skill `/rerail-bug-report` follows, inside the installed package. */
export function bugReportSkillPath(packageRoot: string): string {
  return `${packageRoot}/skills/rerail-bug-report/SKILL.md`;
}

/**
 * `/rerail-bug-report`: work out whether a problem is an engine bug, a shipped rule's or the person's own rule's. The skill
 * in the package holds the steps; this keeps the commands and the promise that matters in the command itself: an issue is
 * public, so sending anything is the person's to decide.
 */
export function bugReportInstructions({ cli, packageRoot, request, shell }: BugReportInstructionsInput): string {
  return [
    `Work out what went wrong with Rerail, then fix it or report it. The person's words: ${request}`,
    "",
    `Read ${bugReportSkillPath(packageRoot)} and follow it.`,
    `Wherever it says \`rerail\`, run \`${cli}\`.`,
    "",
    "- An engine bug, or a bug in a rule that ships with Rerail: offer to report it in an issue on neralabs/rerail.",
    "- A bug in the person's own rule: help them fix it.",
    "",
    "The commands it uses:",
    "",
    commandFence(shell),
    `${cli} support-bundle --json`,
    `${cli} support-bundle --json --session <id>`,
    `${cli} update check`,
    `${cli} rules test <draft.json> --json`,
    "```",
    "",
    "If this is a security vulnerability, it does not go in an issue: tell the person to follow SECURITY.md instead.",
    "Never upload, post, send or paste the support bundle or its contents anywhere, and never open the issue yourself, unless the person asks you to in this session.",
    "Never put in the bundle anything the command did not write.",
  ].join("\n");
}
