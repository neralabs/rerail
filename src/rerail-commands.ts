/**
 * The Rerail commands every coding agent gets, and the instructions each one carries. Claude Code and
 * Cursor read them as slash-command files, Codex as skills (`$rerail-ui`), and OMP and Pi register them
 * in-process (`rerail-command.ts`). One catalog keeps the names, descriptions and `/rerail` list the same
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
  { name: "rerail-stop", description: "Stop Rerail in this checkout; rules and history are kept", summary: "stop Rerail in this repo" },
  { name: "rerail-start", description: "Start Rerail in this checkout, setting the repo up the first time", summary: "start Rerail in this repo" },
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
  /** The shell-quoted launcher the agent runs, e.g. `'/usr/bin/node' '/home/me/.rerail/current/dist/cli.js'`. */
  cli: string;
  /** Where the installed package keeps example code rules and the rule grammar. */
  packageRoot: string;
  /** The person's sentence; a placeholder such as `$ARGUMENTS` when the coding agent substitutes it. */
  request: string;
}

/**
 * `/rerail-create` "in your own words": the coding agent drafts the rule, tests it before
 * showing anything, reads it back in six rows, and adds it only once the person says yes.
 */
export function createRuleInstructions({ cli, packageRoot, request }: CreateRuleInstructionsInput): string {
  const examples = `${packageRoot}/packs/complexity/rules and ${packageRoot}/packs/tslint/rules`;
  const grammar = `${packageRoot}/code-rule-dsl/capabilities.json`;
  return [
    `Create a Rerail rule for this repo from the person's own words: ${request}`,
    "",
    "If there are no words after the command, ask the person in one line what the rule should enforce, then carry on.",
    "",
    "1. Draft the rule. Do not show it yet.",
    "   - A code rule checks code in files, for example \"use design tokens, not hex colours\". It is a `rerail.policy/v1` rule, kind `code`.",
    `     Copy the shape of the examples in ${examples}; the full grammar is in ${grammar}.`,
    "   - A conversation rule checks a command the coding agent runs or something said in the session, for example \"never force-push\".",
    "     It is kind `conversation`: for a command, a structural rule with `match.field` `input.command` and one `regex`, `sample.source` `tool:<tool name>` and `sample.boundary` `tool-call`.",
    "   - Give it the id `local:<short-kebab-name>`.",
    "   - What happens when it is broken, as the rule's top-level `mode`: Steer (`steer`) when you are sure of the rule; Log (`log`) when you are not. Never use `block`.",
    "   - How often (conversation rules): `repeat.mode` `always` for commands, otherwise `once`.",
    "",
    "2. Test it before showing anything. Write the draft to a temporary JSON file as `{ \"kind\": …, \"rule\": …, \"cases\": { \"caught\": […], \"passed\": […] } }`,",
    "   with a few cases it must catch and a few it must let through, taken from this repo's own code where you can",
    "   (a code rule case is `{ \"path\", \"content\" }`; a conversation rule case is `{ \"source\", \"boundary\", \"text\", \"input\" }`). Then run:",
    "",
    "```sh",
    `${cli} rules test <draft.json> --json`,
    "```",
    "",
    "   Fix the draft until it is valid and every case comes out right. You are not sure of the rule when it matches nothing in this repo,",
    "   or hundreds of places, or when the person's words can be read two ways. If it cannot be checked mechanically (\"write clean code\"),",
    "   say \"I can't check this reliably\", offer a narrower version, and add nothing.",
    "",
    "3. Read it back, exactly in this form, and ask once:",
    "",
    "```text",
    "Here is the rule as I understand it.",
    "",
    "  What it checks   <in plain words>",
    "  Where it runs    When I edit a file under <folder> | When I run <command> | When you or I say <…>",
    "  Repo             <this repo's name> only",
    "  What happens     Steer: I'm told in the session and fix it. I'm not stopped.",
    "  How often        Every time | The first time only",
    "  Tested           Would have caught <N> places in this repo",
    "",
    "Add this rule?",
    "  1. Yes, add it",
    "  2. Change something",
    "  3. Show me the rule",
    "  4. Cancel",
    "```",
    "",
    "   For Log, the What happens row reads \"Log: Rerail keeps a record. I'm not told.\". If you are not sure of one row, mark it",
    "   `Not sure`, show one line the rule catches and one it misses from this repo, and set it to Log.",
    "   \"Change something\" offers Steer and Log, each with what it means. \"Show me the rule\" prints the rule JSON, then asks again.",
    "",
    "4. Only after the person says yes, run:",
    "",
    "```sh",
    `${cli} rules add <draft.json>`,
    "```",
    "",
    "   Then reply in one line, `● <what it checks> is on, set to <Steer|Log>.`, followed by the link the command printed.",
    "",
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
