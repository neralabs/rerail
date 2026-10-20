/**
 * Page-side derivations from a rule as `poll.js` maps it from `GET /api/repository/rules`. Nothing
 * here is persisted; the rule document, its effective `mode`, `origin`, `source` and the deciders are the inputs.
 */
import { collapse } from "./format.js";

/** `origin`: "rerail" for rules that come with Rerail ("By Rerail"), "user" for rules created by you (the library). */
export const isByRerail = stored => stored.origin === "rerail";
export const originLabel = stored => (isByRerail(stored) ? "By Rerail" : "Created by you");

/** Pack category as the origin chip names it: "By Rerail · Language", "By Rerail · Repository". */
const CATEGORY_LABEL = { language: "Language", framework: "Framework", repository: "Repository" };
export const categoryLabel = category => CATEGORY_LABEL[category] ?? "";

export const folderText = folder => (folder === "" ? "Whole repository" : folder);

/**
 * Whether the rule runs: switched on, and its options apply to its document. A rule whose stored options no longer
 * apply (`configError`) is left out of the compiled set, so it is not counted as on.
 */
export const isRunning = stored => stored.enabled && !stored.configError;

/**
 * The rule panel's On row: on or off with what decided it, each folder its own when they disagree; then, for a rule
 * switched on somewhere whose options don't apply, that it runs nowhere and why.
 */
export function onText(stored, checkoutPath) {
  const switched = stored.mixed
    ? stored.folderStates.map(state => `${state.on ? "On" : "Off"} in ${folderText(state.folder)}`).join(", ")
    : `${stored.enabled ? "On" : "Off"} · ${deciderText(stored.onSource, checkoutPath)}`;
  const stopped = stored.configError && stored.folderStates.some(state => state.on);
  return stopped ? `${switched} · not running: ${stored.configError}` : switched;
}

/**
 * What decided a rule's on, mode, suppressions or an option, in words (`Decider` from the API). `checkoutPath` is the
 * page's own checkout, which reads as "this worktree".
 */
export function deciderText(decider, checkoutPath) {
  const at = "folder" in decider && decider.folder !== "" ? ` at ${decider.folder}` : "";
  const forPack = decider.target === "pack" ? " for its rule pack" : "";
  switch (decider.by) {
    case "checkout": return `Set in ${decider.checkoutPath === checkoutPath ? "this worktree" : decider.checkoutPath}${at}${forPack}`;
    case "repository": return `Set for the repository${at}${forPack}`;
    case "pack-start": return `Rule pack ${decider.packId} default`;
    default: return "The rule's default";
  }
}

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

export const SUPPRESSIONS_LABEL = { honour: "Honoured", ignore: "Ignored" };

/**
 * Whether linter suppression comments (`// eslint-disable-next-line`, `# noqa`) silence the rule, and what decided
 * it; each folder its own when the folders the rule is added at disagree.
 */
export function suppressionsText(stored, checkoutPath) {
  const states = stored.folderStates;
  if (states.some(state => state.suppressions !== states[0].suppressions)) {
    return states.map(state => `${SUPPRESSIONS_LABEL[state.suppressions] ?? state.suppressions} in ${folderText(state.folder)}`).join(", ");
  }
  return `${SUPPRESSIONS_LABEL[stored.suppressions] ?? stored.suppressions} · ${deciderText(stored.suppressionsSource, checkoutPath)}`;
}

/** What the suppression comments switch sets: honour once every folder ignores them, else ignore. */
export const nextSuppressions = stored => (stored.folderStates.every(state => state.suppressions === "ignore") ? "honour" : "ignore");

/**
 * Whether a setting on `target` ("rule" or "pack"), written as `owner` ("repository" or "checkout", this worktree being
 * `checkoutPath`) at the folder `state` is added at, would decide that folder's suppressions. It mirrors the
 * resolver's precedence: this worktree, then the worktrees containing it, then the repository; at one owner the deeper
 * folder; at one owner and folder the rule's own setting before its pack's. Neither a pack's starting value nor a
 * rule's default holds it off.
 */
export function settingDecides(state, target, owner, checkoutPath) {
  const source = state.suppressionsSource;
  if (source?.by !== "checkout" && source?.by !== "repository") return true;
  // Lower outranks: this worktree, a worktree containing it, the repository.
  const ownerRank = owner === "repository" ? 2 : 0;
  const sourceRank = source.by === "repository" ? 2 : source.checkoutPath === checkoutPath ? 0 : 1;
  if (ownerRank !== sourceRank) return ownerRank < sourceRank;
  if (state.folder.length !== source.folder.length) return state.folder.length > source.folder.length;
  return target === "rule" || source.target === "pack";
}

/** Whether pack `packId`'s setting would decide `state`'s suppressions: only where the pack added the rule. */
export function packSettingDecides(state, packId, owner, checkoutPath) {
  return state.source?.kind === "pack" && state.source.id === packId && settingDecides(state, "pack", owner, checkoutPath);
}

/** Whether a stored rule is in pack `packId` here: the pack added it at one of its folders. */
export const inPack = (rule, packId) => rule.folderStates.some(state => state.source?.kind === "pack" && state.source.id === packId);

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

/**
 * The rule's configured options as `name: value`, each with what decided it, and why they don't apply when they
 * don't; `null` when none is configured.
 */
export function optionsText(stored, checkoutPath) {
  const options = Object.entries(stored.config ?? {});
  if (options.length === 0) return null;
  const text = options.map(([name, value]) => {
    const decider = stored.configSource?.[name];
    return `${name}: ${JSON.stringify(value)}${decider === undefined ? "" : ` · ${deciderText(decider, checkoutPath)}`}`;
  }).join(", ");
  return stored.configError ? `${text} · does not apply: ${stored.configError}` : text;
}

/** Conversation rules are in internal testing (README.md, Internal testing): Rerail 0.1.0 releases code rules only. */
export const INTERNAL_TESTING = "Internal testing";
export const isInternalTesting = stored => stored.kind === "conversation";
export const kindLabel = stored => (isInternalTesting(stored) ? `Conversation rule · ${INTERNAL_TESTING}` : "File rule");

/** What a conversation rule samples, as the scope sentence names it. */
function sampleSubject(source) {
  if (source === "user-input") return "what you write";
  if (source === "assistant-text") return "what the coding agent writes";
  if (source === "tool:*") return "every tool call";
  return `${source.slice(5)} tool calls`;
}

/** "always" / "once" / "after 3 turns", as the technical disclosure reads a repeat policy. */
function repeatText(repeat) {
  if (!repeat) return "always";
  if (repeat.mode === "after-gap") return `after ${repeat.gapTurns} turns`;
  return repeat.mode;
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

/** Technical facts for the panel's disclosure. */
export function technical(stored) {
  const rule = stored.rule;
  if (stored.kind === "conversation") {
    const tokens = rule.sample.minimumTokens ? ` · ≥${rule.sample.minimumTokens} tokens` : "";
    return {
      rows: [
        ["Kind", kindLabel(stored)],
        ["Match", `${rule.match.operator ?? "pattern"}${rule.match.threshold === undefined ? "" : ` ${rule.match.threshold}`}`],
        ["Sample", `${rule.sample.source} · ${rule.sample.boundary}${tokens}`],
        ["Repeat", repeatText(rule.repeat)],
        ["Rule id", rule.id],
        ["Revision", String(stored.revision)],
      ],
      pattern: JSON.stringify(rule.match, null, 2),
    };
  }
  const clause = rule.clauses[0];
  const form = clause ? ["forbid", "require_each", "budget", "llm"].find(f => clause[f] !== undefined) : undefined;
  return {
    rows: [
      ["Kind", "File rule"],
      ["Clauses", rule.clauses.map(c => c.id).join(", ")],
      ["Rule id", rule.id],
      ["Revision", String(stored.revision)],
    ],
    pattern: clause && form ? JSON.stringify(clause[form], null, 2) : "",
  };
}
