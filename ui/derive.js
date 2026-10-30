/**
 * Page-side derivations from a rule as `poll.js` maps it from `GET /api/repository/rules`. Nothing
 * here is persisted; the rule document, its effective `mode`, `origin`, `source` and per-folder states are the inputs.
 */
import { collapse } from "./format.js";

/** `origin`: "pack" for rules that come in an installed rule pack, "user" for rules created by you (the library). */
export const isFromPack = stored => stored.origin === "pack";

/**
 * Whether a pack (`GET /api/rule-packs`) is one of Rerail's own, as published: any other publisher is a third party
 * Rerail has not reviewed, and a pack installed from a folder or tarball (`installedFrom`) is whatever that folder
 * says it is.
 */
export const isRerailPack = pack => pack.publisher === "rerail" && pack.installedFrom === undefined;
/** Whether a pack is the checkout's own rules (`.rerail/rules`): never installed or added, it runs in that worktree alone. */
export const isWorktreePack = pack => pack.publisher === "worktree";
/**
 * Who publishes a pack, as the page names it: "By Rerail", "In this worktree", "By @acme · not reviewed", and for one
 * installed from a folder or tarball "From a folder · not reviewed" (a third party's: "By @acme · From a folder · not
 * reviewed"). A third party is always marked as one, so a scope that reads like Rerail's (`@rerail`) is never mistaken
 * for it, and a folder never passes for Rerail's published pack whatever its package.json says.
 */
export const publisherLabel = pack => {
  if (isWorktreePack(pack)) return "In this worktree";
  if (isRerailPack(pack)) return "By Rerail";
  const by = pack.publisher === "rerail" ? "" : `By ${pack.publisher} · `;
  return pack.installedFrom === undefined ? `${by}not reviewed` : `${by}From a ${pack.installedFrom} · not reviewed`;
};
/** A pack as a rule's origin names it: its name, with its publisher when that is a third party ("Acme SQL · By @acme · not reviewed"). */
export const packOriginName = pack => (isRerailPack(pack) || isWorktreePack(pack) ? pack.name : `${pack.name} · ${publisherLabel(pack)}`);

/** Whether a rule (or a pack, from `GET /api/rule-packs`) waits for a person's approval, and runs in Log until then. */
export const awaitingApproval = item => item.approval?.approved === false;

/** The origin of a rule that came in a rule pack this page does not know (its pack was removed): no publisher is claimed. */
export const UNKNOWN_PACK_ORIGIN = "From a rule pack";

/**
 * The files of the worktree at `checkoutPath` in its `.rerail/rules` that were left out (`GET /api/rule-packs?checkout=`
 * `skipped`), each by its name in the folder (the folder's own name when the whole folder was), with why.
 */
export function worktreeLeftOut(skipped, checkoutPath) {
  if (checkoutPath === undefined || checkoutPath === null) return [];
  const slashed = path => path.replaceAll("\\", "/");
  const folder = `${slashed(checkoutPath).replace(/\/$/u, "")}/.rerail/rules`;
  return skipped.flatMap(({ path, reason }) => {
    const at = slashed(path);
    if (at === folder) return [{ file: ".rerail/rules", reason }];
    return at.startsWith(`${folder}/`) ? [{ file: at.slice(folder.length + 1), reason }] : [];
  });
}

/** Pack category as the origin chip names it: "By Rerail · Language", "By Rerail · Repository". */
const CATEGORY_LABEL = { language: "Language", framework: "Framework", repository: "Repository" };
export const categoryLabel = category => CATEGORY_LABEL[category] ?? "";

/**
 * Whether the rule runs: switched on, and its options apply to its document. A rule whose stored options no longer
 * apply (`configError`) is left out of the compiled set, so it is not counted as on.
 */
export const isRunning = stored => stored.enabled && !stored.configError;

const titles = new WeakMap();
/** Sentence the row leads with: conversation rule → reference; file rule → first clause's message, else its id. */
export function ruleTitle(stored) {
  let title = titles.get(stored);
  if (title === undefined) {
    const rule = stored.rule;
    const clause = stored.kind === "conversation" ? undefined : rule.clauses[0];
    title = collapse(stored.kind === "conversation" ? rule.reference : clause?.violation?.message) || rule.id;
    titles.set(stored, title);
  }
  return title;
}

/** `rule.id` before the first `:`; the whole id when it has none. */
export function rulePrefix(id) {
  const i = id.indexOf(":");
  return i < 0 ? id : id.slice(0, i);
}

/** Every rule, either kind, carries one mode: what Rerail does on a violation. */
export const ruleMode = stored => stored.rule.mode;

export const MODE_LABEL = { log: "Log", steer: "Steer", block: "Block" };
export const modeLabel = stored => MODE_LABEL[ruleMode(stored)] ?? ruleMode(stored);
export const isLog = stored => ruleMode(stored) === "log";
/** How Rerail handled one past violation: by the mode recorded with it, since the rule may have changed mode. Block shows as Steered (not in this version). */
export const violationOutcome = finding => (finding.mode === "log" ? "logged" : "steered");

/** One sentence per mode, as the rule panel explains it. */
export const MODE_PLAIN = {
  log: "Rerail records the violation for you. Your coding agent isn’t told.",
  steer: "Rerail tells the coding agent as it works, and the coding agent fixes it before moving on.",
  block: "Rerail stops the coding agent before it acts.",
};

/** Whether a stored rule is in pack `packId` here: the pack added it at one of its folders. */
export const inPack = (rule, packId) => rule.folderStates.some(state => state.source?.kind === "pack" && state.source.id === packId);

/**
 * Whether pack `packId`'s setting, written as `owner` ("repository" or "checkout", this worktree being
 * `checkoutPath`) at the folder `state` is added at, would decide that folder's suppressions: only where the pack
 * added the rule, and by the resolver's precedence: this worktree, then the worktrees containing it, then the
 * repository; at one owner the deeper folder; at one owner and folder a rule's own setting before its pack's. Neither
 * a pack's starting value nor a rule's default holds it off.
 */
export function packSettingDecides(state, packId, owner, checkoutPath) {
  if (state.source?.kind !== "pack" || state.source.id !== packId) return false;
  const source = state.suppressionsSource;
  if (source?.by !== "checkout" && source?.by !== "repository") return true;
  // Lower outranks: this worktree, a worktree containing it, the repository.
  const ownerRank = owner === "repository" ? 2 : 0;
  const sourceRank = source.by === "repository" ? 2 : source.checkoutPath === checkoutPath ? 0 : 1;
  if (ownerRank !== sourceRank) return ownerRank < sourceRank;
  if (state.folder.length !== source.folder.length) return state.folder.length > source.folder.length;
  return source.target === "pack";
}

/**
 * How pack `packId`'s rules here treat suppression comments, from each rule's own value, and what the pack's switch
 * (written as `owner` from the worktree `checkoutPath`) sets: honour once every folder of its rules it decides ignores
 * them, else ignore. A folder the pack did not add the rule at, or a setting outranking the pack's decides, counts in
 * the text but not in the switch, which does not change it.
 */
export function packSuppressions(rules, packId, owner, checkoutPath) {
  const ignoring = rules.filter(r => r.folderStates.some(state => state.suppressions === "ignore")).length;
  const text = ignoring === 0 ? "honoured by its rules"
    : ignoring === rules.length ? "ignored by its rules"
    : `ignored by ${ignoring} of its ${rules.length} rules`;
  const decided = rules.flatMap(rule => rule.folderStates.filter(state => packSettingDecides(state, packId, owner, checkoutPath)));
  return { text, next: decided.length > 0 && decided.every(state => state.suppressions === "ignore") ? "honour" : "ignore" };
}

/** Conversation rules are in internal testing (README.md, Internal testing): Rerail 0.1.0 releases code rules only. */
export const INTERNAL_TESTING = "Internal testing";
export const isInternalTesting = stored => stored.kind === "conversation";

/** What a conversation rule samples, as the scope sentence names it. */
function sampleSubject(source) {
  if (source === "user-input") return "what you write";
  if (source === "assistant-text") return "what the coding agent writes";
  if (source === "tool:*") return "every tool call";
  return `${source.slice(5)} tool calls`;
}

/** Where the rule applies, as a sentence. */
export function scopeText(stored) {
  if (stored.kind === "conversation") {
    const { source, boundary } = stored.rule.sample;
    return `Watches ${sampleSubject(source)}, one ${boundary === "tool-call" ? "call" : boundary} at a time.`;
  }
  const scope = stored.rule.scope;
  if (!scope) return "Every file in the repository.";
  const inc = scope.include.join(", ");
  const exclude = scope.exclude ?? [];
  if (exclude.length === 0) return inc;
  const shown = exclude.slice(0, 3).join(", ");
  return exclude.length > 3 ? `${inc}, except ${shown} and ${exclude.length - 3} more` : `${inc}, except ${shown}`;
}

/** The condition in words: a file rule's description, a conversation rule's reference or message. */
export function conditionText(stored) {
  const rule = stored.rule;
  if (stored.kind === "conversation") return collapse(rule.reference || rule.description || "");
  return collapse(rule.description ?? "");
}

/** What the coding agent is told (Steer) or what is recorded (Log), before any violation exists. */
export function messageText(stored) {
  if (stored.kind === "conversation") return stored.rule.action.content;
  return stored.rule.clauses[0]?.violation?.message ?? "";
}
