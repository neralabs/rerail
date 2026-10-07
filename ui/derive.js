/**
 * Page-side derivations from a stored rule (`GET /api/rules` row shape). Nothing here is
 * persisted; the rule document and `catalogOrigin` are the only inputs.
 */
import { collapse } from "./format.js";

/**
 * `catalogOrigin` enum in scoped-rule-catalog.ts / schema.ts is ["local", "bundled", "nera"].
 * `nera` (a rule synced down from the Nera service) gets its own label and rides with
 * "your rules" in origin grouping.
 */
export const ORIGIN = {
  local: { cls: "mine", label: "Added by you", chip: "added" },
  bundled: { cls: "preset", label: "Nera preset", chip: "preset" },
  nera: { cls: "synced", label: "Synced from Nera", chip: null },
};
export const originOf = stored => ORIGIN[stored.catalogOrigin] ?? { cls: "", label: stored.catalogOrigin, chip: null };
export const isPreset = stored => stored.catalogOrigin === "bundled";

const titles = new WeakMap();
/** Sentence the row leads with: cosine → reference; policy → first check's message, else its id. */
export function ruleTitle(stored) {
  let title = titles.get(stored);
  if (title === undefined) {
    const rule = stored.rule;
    const check = stored.kind === "cosine" ? undefined : rule.checks[0];
    title = stored.kind === "cosine" ? collapse(rule.reference) : collapse(check?.finding?.message || check?.id || rule.id);
    titles.set(stored, title);
  }
  return title;
}

/** `rule.id` before the first `:`; the whole id when it has none. */
export function rulePrefix(id) {
  const i = id.indexOf(":");
  return i < 0 ? id : id.slice(0, i);
}

const SEVERITY_RANK = { error: 3, warning: 2, advisory: 1 };

/** cosine → its delivery; policy → the worst severity across its checks. */
export function ruleMode(stored) {
  if (stored.kind === "cosine") return stored.rule.action.delivery;
  let worst = "advisory";
  for (const check of stored.rule.checks) {
    const s = check.finding?.severity ?? "advisory";
    if ((SEVERITY_RANK[s] ?? 0) > (SEVERITY_RANK[worst] ?? 0)) worst = s;
  }
  return worst;
}

/** What Rerail does with a trigger, in the design's words. Advisory findings and `observe` never steer. */
export const MODE_LABEL = {
  steer: "Steers the agent mid-turn",
  nextTurn: "Reminds the agent on its next turn",
  followUp: "Follows up after the turn",
  observe: "Flags in review only",
  error: "Steers the agent mid-turn",
  warning: "Steers the agent mid-turn",
  advisory: "Flags in review only",
};
export const modeLabel = stored => MODE_LABEL[ruleMode(stored)] ?? ruleMode(stored);
export const isLog = stored => { const m = ruleMode(stored); return m === "observe" || m === "advisory"; };

export const kindLabel = stored => (stored.kind === "cosine" ? "Conversation rule" : "Code rule");

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
  if (stored.kind === "cosine") {
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
  if (stored.kind === "cosine") {
    const tokens = rule.sample.minimumTokens ? ` · ≥${rule.sample.minimumTokens} tokens` : "";
    return {
      rows: [
        ["Kind", "cosine"],
        ["Match", `${rule.match.operator} ${rule.match.threshold}`],
        ["Sample", `${rule.sample.source} · ${rule.sample.boundary}${tokens}`],
        ["Delivery", rule.action.delivery],
        ["Repeat", repeatText(rule.repeat)],
        ["Revision", String(stored.revision)],
      ],
      pattern: rule.action.content,
    };
  }
  const check = rule.checks[0];
  const form = check ? ["forbid", "require_each", "budget", "llm"].find(f => check[f] !== undefined) : undefined;
  return {
    rows: [
      ["Kind", "policy"],
      ["Checks", rule.checks.map(c => c.id).join(", ")],
      ["Form", form ?? "—"],
      ["Severity", ruleMode(stored)],
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
