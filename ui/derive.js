/**
 * Page-side derivations from a rule as `toStoredRules` maps it from `GET /api/repository/rules`. Nothing
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

/**
 * Whether a rule is one created by you (the library) that waits for your approval: `/rerail-create` adds the rules it
 * finds there, unapproved, so they run in Log until you approve them. A worktree's own rules and packs Rerail did not
 * publish are not: someone else wrote those, and each is read on its own page before it is approved. Nor is an edited
 * copy (of a rule Rerail ships, or of your approved rule a coding agent edited): approving it turns the original off
 * where the copy is added, which its page says.
 */
export const awaitingYourApproval = stored => awaitingApproval(stored) && !isFromPack(stored) && stored.approval.subject.kind === "rule"
  && !stored.copiedFromRuleId;

/** Whether a rule is an edited copy of another (of a rule Rerail ships, or of your approved rule), saved in your library. */
export const isEditedCopy = stored => Boolean(stored.copiedFromRuleId) && !isFromPack(stored);

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
 * Whether the rule runs: switched on, its options apply to its document, and Rerail runs that document. A rule whose
 * stored options no longer apply (`configError`), or that Rerail keeps but never evaluates (`inactive`: a language not
 * read yet, or a pack version's form this Rerail no longer runs), is left out of the compiled set, so it is not counted
 * as on.
 */
export const isRunning = stored => stored.enabled && !stored.configError && !stored.inactive;

/** Why a rule switched on runs nowhere, in a sentence; null when it runs, or is off. */
export function notRunningReason(stored) {
  if (!stored.enabled) return null;
  if (stored.configError) return `Its options don't apply: ${stored.configError}`;
  return stored.inactive?.reason ?? null;
}

const titles = new WeakMap();
/**
 * Sentence the row leads with: the rule's title; else a conversation rule's reference, a file rule's first clause's
 * message, a `.rerail` rule's description, else its id.
 */
export function ruleTitle(stored) {
  let title = titles.get(stored);
  if (title === undefined) {
    title = collapse(stored.rule.title) || collapse(untitledLead(stored)) || stored.rule.id;
    titles.set(stored, title);
  }
  return title;
}

/** What a rule with no title is led by: a conversation rule's reference, a JSON code rule's first message, a `.rerail` rule's description. */
function untitledLead(stored) {
  if (stored.kind === "conversation") return stored.rule.reference;
  // A `.rerail` rule's messages are written in its file, which the dashboard is not given clause by clause.
  if (stored.kind === "rerail") return stored.rule.description;
  return stored.rule.clauses[0]?.violation?.message;
}

/** `rule.id` before the first `:`; the whole id when it has none. */
export function rulePrefix(id) {
  const i = id.indexOf(":");
  return i < 0 ? id : id.slice(0, i);
}

/** Every rule, either kind, carries one mode: what Rerail does on a violation. */
export const ruleMode = stored => stored.rule.mode;

/** A mode as the dashboard names it. Block steers until blocking ships, so it says so wherever a rule is on it. */
export const MODE_LABEL = { log: "Log", steer: "Steer", block: "Block (steers for now)" };
export const modeLabel = stored => MODE_LABEL[ruleMode(stored)] ?? ruleMode(stored);
export const isLog = stored => ruleMode(stored) === "log";
/**
 * How Rerail handled one past violation, as recorded then (the rule may have changed mode since): logged when the
 * audit ran in Log mode or the rule was in Log mode, the same `logged` the session row's steered count reads. Block
 * shows as Steered (not in this version).
 */
export const violationOutcome = finding => (finding.logged ? "logged" : "steered");

/** One sentence per mode, as the rule panel explains it. */
export const MODE_PLAIN = {
  log: "Rerail only records it, for you to review. Your coding agent isn’t told.",
  steer: "Rerail tells your coding agent as it works, so it fixes it.",
  block: "Block is not in this release. Rerail steers instead.",
};

/** Whether a stored rule is in pack `packId` here: the pack added it at one of its folders. */
export const inPack = (rule, packId) => rule.folderStates.some(state => state.source?.kind === "pack" && state.source.id === packId);

/**
 * How a pack's rules here treat linter suppression comments, from each rule's own value: honoured, ignored, or ignored
 * by some of them. The page only says so; it has no switch for it.
 */
export function packSuppressions(rules) {
  const ignoring = rules.filter(r => r.folderStates.some(state => state.suppressions === "ignore")).length;
  if (ignoring === 0) return "honoured by its rules";
  return ignoring === rules.length ? "ignored by its rules" : `ignored by ${ignoring} of its ${rules.length} rules`;
}

/** Conversation rules are in internal testing (README.md, Internal testing): Rerail's latest release ships code rules only. */
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

/** What the coding agent is told (Steer) or what is recorded (Log), before any violation exists; for a `.rerail` rule, only its violations say. */
export function messageText(stored) {
  if (stored.kind === "conversation") return stored.rule.action.content;
  if (stored.kind === "rerail") return "";
  return stored.rule.clauses[0]?.violation?.message ?? "";
}

/**
 * One row per rule id, in the renderers' shape: on (`enabled`, the switch) only where it is on in every folder it is
 * added at, the first folder's mode, suppressions and options on the rule, and each folder's own on, mode,
 * suppressions and what decided them (`folderStates`; `mixed` when on or mode differ). `configError` is why the rule's
 * options don't apply, from any folder, and `inactive` why Rerail never evaluates its document: the rule then runs
 * nowhere, whatever its switch says. A change from the row is written at every folder.
 */
export function toStoredRules(entries) {
  const rules = new Map();
  for (const entry of entries) {
    const id = entry.rule.id;
    const known = rules.get(id);
    if (known === undefined) rules.set(id, storedRule(entry));
    else addFolder(known, entry);
  }
  return rules;
}

/** Adds `entry`'s folder to `known`, the row of the same rule from an earlier folder; once per folder. */
function addFolder(known, entry) {
  if (known.folders.includes(entry.folder)) return;
  known.folders.push(entry.folder);
  known.folderStates.push(folderState(entry));
  known.enabled = known.enabled && entry.on;
  known.configError ??= entry.configError ?? null;
  known.inactive ??= entry.inactive ?? null;
  known.mixed = known.folderStates.some(state => state.on !== entry.on || state.mode !== entry.mode) || known.mixed;
  known.addedAt = earliest(known.addedAt, entry.addedAt ?? null);
  known.updatedAt = latest(known.updatedAt, entry.updatedAt ?? null);
}

/** The row of a rule from the first folder `entry` adds it at. */
function storedRule(entry) {
  return {
    kind: entry.kind,
    rule: { ...entry.rule, mode: entry.mode },
    enabled: entry.on,
    origin: entry.source.kind === "library_rule" ? "user" : "pack",
    source: entry.source,
    folders: [entry.folder],
    folderStates: [folderState(entry)],
    mixed: false,
    onSource: entry.onSource,
    modeSource: entry.modeSource,
    suppressions: entry.suppressions,
    suppressionsSource: entry.suppressionsSource,
    config: entry.config,
    configSource: entry.configSource,
    configError: entry.configError ?? null,
    inactive: entry.inactive ?? null,
    revision: entry.revision,
    addedAt: entry.addedAt ?? null,
    // Present for a rule Rerail did not publish: until `approved`, it runs in Log (`mode` says so).
    approval: entry.approval ?? null,
    // An edited copy (`<id>-copy`, of a rule Rerail ships or of your approved rule): the rule it was copied from.
    copiedFromRuleId: entry.copiedFromRuleId ?? null,
    updatedAt: entry.updatedAt ?? null,
  };
}

/** When a rule present in several folders was first added: the earliest folder's; `null` where none says. */
const earliest = (a, b) => (a === null ? b : b === null ? a : Math.min(a, b));
/** When its settings last changed: the latest folder's. */
const latest = (a, b) => (a === null ? b : b === null ? a : Math.max(a, b));

/** One folder's own on, mode and suppressions of a rule, what decided its suppressions, and what added it there. */
const folderState = entry => ({
  folder: entry.folder, on: entry.on, mode: entry.mode, suppressions: entry.suppressions, suppressionsSource: entry.suppressionsSource,
  source: entry.source,
});

/**
 * A rule's distinct violations in a range (one `rules_activity` entry), each in exactly one outcome: Log only (recorded
 * on Log, so the coding agent was not told), Declined (a steer the agent declined with `rerail feedback`, even when
 * another audit recorded it on Log), or Steered (sent to the agent, and not declined). History by rule's chips and
 * Overview's Violations steered both read it. An answer without the declined counts reads none declined.
 * @param {{ distinctViolations: number, loggedViolations?: number, declinedViolations?: number, declinedLoggedViolations?: number }} a
 * @returns {{ all: number, logged: number, steered: number, declined: number }}
 */
export function violationOutcomes(a) {
  const all = a.distinctViolations;
  const declined = Math.min(a.declinedViolations ?? 0, all);
  const logged = Math.max(0, Math.min((a.loggedViolations ?? 0) - (a.declinedLoggedViolations ?? 0), all - declined));
  return { all, logged, steered: all - logged - declined, declined };
}
