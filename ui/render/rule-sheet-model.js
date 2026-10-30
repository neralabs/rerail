/**
 * What the rule sheet says, without the DOM: the "Edit with coding agent" prompts, the "What the agent sees" hand-off,
 * dates with the year, which rules the sheet can delete and what deleting one does. Pure functions only.
 */

/** The prompt box's chips: the sheet's own section names; the box title carries the verb. */
export const ASK = [["ask", "Ask"], ["check", "What it checks"], ["scope", "Where it applies"], ["delivery", "On a violation"]];
/** Prompts that end mid-sentence: the person finishes them after pasting. */
export const ASK_OPEN = { check: true, scope: true, delivery: true };

const withStop = text => (/[.!?]$/u.test(text) ? text : `${text}.`);

/**
 * The prompt for chip `intent` about rule `rule` (`{ id, title, scope, mode }`, `mode` as the sheet labels it).
 * Every prompt names the rule by id, so the coding agent's Rerail skill acts on the right one.
 */
export function askPrompt(rule, intent) {
  const ref = `the rule "${rule.id}" (${rule.title})`;
  if (intent === "check") return `Using Rerail, update ${ref}. Show me the change before saving. I want it to check: `;
  if (intent === "scope") return `Using Rerail, update ${ref}. Currently it applies to: ${withStop(rule.scope)} Show me the change before saving. I want it to apply to: `;
  if (intent === "delivery") return `Using Rerail, update ${ref}. Currently, on a violation: ${rule.mode}. Show me the change before saving. Instead I want it to: `;
  return `Using Rerail, explain ${ref}: what it checks, where it applies, and its most recent violations.`;
}

/** The hand-off for "What the agent sees": the coding agent rewrites the rule's message; the page writes nothing. */
export const agentSeesPrompt = (id, text) =>
  `Using Rerail, update the rule "${id}": change the message the coding agent sees on a violation to: ${text.trim()}`;

/** Why a rule on Log's message goes nowhere: under the message, and under its draft. */
export const LOG_NOTE = "Not sent while this rule is on Log.";

/** What saving a new message does: a By Rerail rule's edit is a library copy, and on Log nothing reaches the agent. */
export const agentSeesHint = ({ byRerail, log }) => [
  ...(byRerail ? ["Saved as your copy. This one turns off."] : []),
  log ? LOG_NOTE : "Reaches open sessions once your agent saves it.",
].join(" ");

const DAY_FORMAT = new Intl.DateTimeFormat("en-US", { day: "numeric", month: "short", year: "numeric" });

/** An absolute day with its year ("Sep 3, 2026"); "–" when the daemon has none. */
export const fmtDay = t => (t === null || t === undefined ? "–" : DAY_FORMAT.format(new Date(t)));

/** The attachments that add rule `stored` to this repository on its own, as a library rule, one per folder. */
export const libraryAttachments = stored => stored.folderStates
  .filter(state => state.source?.kind === "library_rule")
  .map(state => ({ sourceKind: "library_rule", sourceId: state.source.id, folder: state.folder }));

/** Only a rule created by you, added here on its own, has Delete; a pack's rules are switched, the pack removed. */
export const canDelete = stored => stored.origin === "user" && libraryAttachments(stored).length > 0;

/** What deleting a rule from this repository does, before the second click: `{ repo, pastViolations, on }`. */
export function deleteConsequences({ repo, pastViolations, on }) {
  return [
    `Rerail stops auditing this rule in every session in ${repo}.`,
    ...(pastViolations > 0 ? [`Its ${pastViolations} past ${pastViolations === 1 ? "violation stays" : "violations stay"} in History.`] : []),
    "The rule stays in your library; your coding agent can add it here again.",
    ...(on ? ["To pause it without losing it, turn it off instead."] : []),
  ];
}

/** A violation's path as the sheet shows it: the file, and the folder it is in ("" at the root). */
export function splitPath(path) {
  if (!path) return { file: "", dir: "" };
  const i = path.lastIndexOf("/");
  return i === -1 ? { file: path, dir: "" } : { file: path.slice(i + 1), dir: path.slice(0, i) };
}
