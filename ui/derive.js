/**
 * Page-side derivations from a rule as `poll.js` maps it from `GET /api/repository/rules`. Nothing
 * here is persisted; the rule document, its effective `mode`, `origin` and the deciders are the inputs.
 */
import { collapse } from "./format.js";

/** `origin`: "user" for rules created by you (the library), "rerail" for rules that come with Rerail. */
export const ORIGIN = {
  user: { cls: "mine", label: "Created by you", chip: "added" },
  rerail: { cls: "preset", label: "By Rerail", chip: "preset" },
};
export const originOf = stored => ORIGIN[stored.origin] ?? { cls: "", label: stored.origin, chip: null };
export const isPreset = stored => stored.origin === "rerail";

/** Where a rule comes from, as the panel names it: By Rerail with its rule pack, or Created by you. */
export function sourceText(stored) {
  const { packId } = stored.source;
  return packId === null ? originOf(stored).label : `${originOf(stored).label} · rule pack ${packId}`;
}

export const folderText = folder => (folder === "" ? "Whole repository" : folder);

/**
 * What decided a rule's on or mode, in words (`Decider` from the API). `checkoutPath` is the page's own
 * checkout, which reads as "this worktree".
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
/** Sentence the row leads with: conversation → reference; file → first clause's message, else its id. */
export function ruleTitle(stored) {
  let title = titles.get(stored);
  if (title === undefined) {
    const rule = stored.rule;
    const check = stored.kind === "conversation" ? undefined : rule.clauses[0];
    title = stored.kind === "conversation" ? collapse(rule.reference) : collapse(check?.violation?.message || check?.id || rule.id);
    titles.set(stored, title);
  }
  return title;
}

/** `rule.id` before the first `:`; the whole id when it has none. */
export function rulePrefix(id) {
  const i = id.indexOf(":");
  return i < 0 ? id : id.slice(0, i);
}

/** Every rule, either kind, carries one mode: log, steer, or block. */
export const ruleMode = stored => stored.rule.mode;

/** What Rerail does with a trigger, in the design's words. `log` never steers. */
export const MODE_LABEL = {
  log: "Flags in review only",
  steer: "Steers the agent mid-turn",
  block: "Blocks and asks you first",
};
export const modeLabel = stored => MODE_LABEL[ruleMode(stored)];
export const isLog = stored => ruleMode(stored) === "log";

export const kindLabel = stored => (stored.kind === "conversation" ? "Conversation rule" : "Code rule");

/** What a conversation rule samples, as the scope sentence names it. */
function sampleSubject(source) {
  if (source === "user-input") return "what you write";
  if (source === "assistant-text") return "what the agent writes";
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
    return `Watches ${sampleSubject(source)}, one ${boundary} at a time.`;
  }
  const scope = stored.rule.scope;
  if (!scope) return "Every file in the repository.";
  const inc = scope.include.join(", ");
  return scope.exclude?.length ? `${inc} — except ${scope.exclude.join(", ")}` : inc;
}

/** Technical facts for the panel's disclosure. */
export function technical(stored) {
  const rule = stored.rule;
  if (stored.kind === "conversation") {
    const tokens = rule.sample.minimumTokens ? ` · ≥${rule.sample.minimumTokens} tokens` : "";
    return {
      rows: [
        ["Kind", "cosine"],
        ["Match", `${rule.match.operator} ${rule.match.threshold}`],
        ["Sample", `${rule.sample.source} · ${rule.sample.boundary}${tokens}`],
        ["Mode", rule.mode],
        ["Repeat", repeatText(rule.repeat)],
        ["Revision", String(stored.revision)],
      ],
      pattern: rule.action.content,
    };
  }
  const check = rule.clauses[0];
  const form = check ? ["forbid", "require_each", "budget", "llm"].find(f => check[f] !== undefined) : undefined;
  return {
    rows: [
      ["Kind", "policy"],
      ["Checks", rule.clauses.map(c => c.id).join(", ")],
      ["Form", form ?? "—"],
      ["Mode", rule.mode],
      ["Revision", String(rule.revision)],
    ],
    pattern: check && form ? JSON.stringify(check[form], null, 2) : "",
  };
}

const haystacks = new WeakMap();
/** Text the search box runs over: title, id, description, scope. */
export function searchText(stored) {
  let hay = haystacks.get(stored);
  if (hay === undefined) {
    hay = [ruleTitle(stored), stored.rule.id, stored.rule.description ?? "", scopeText(stored)].join(" ").toLowerCase();
    haystacks.set(stored, hay);
  }
  return hay;
}
