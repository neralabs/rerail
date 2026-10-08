/**
 * Page-side derivations from a stored rule (`GET /api/rules` row shape). Nothing here is
 * persisted; the rule document and `catalogOrigin` are the only inputs.
 */
import { collapse } from "./format.js";

/**
 * `catalogOrigin` enum in schema.ts is ["user", "rerail"]: rules the user wrote or imported,
 * and rules shipped with Rerail.
 */
export const ORIGIN = {
  user: { cls: "mine", label: "Added by you", chip: "added" },
  rerail: { cls: "preset", label: "Nera preset", chip: "preset" },
};
export const originOf = stored => ORIGIN[stored.catalogOrigin] ?? { cls: "", label: stored.catalogOrigin, chip: null };
export const isPreset = stored => stored.catalogOrigin === "rerail";

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

const NEW_WINDOW = 7 * 86_400_000;
export const isNew = stored => Date.now() - stored.createdAt < NEW_WINDOW;
