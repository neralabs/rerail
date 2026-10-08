import { conversationRuleSchema } from "@rerail/rule-schema";
import { codeRuleSchema } from "@rerail/code-rule-schema";
import { ruleListSchema, type PlaygroundRuleSnapshot } from "@protocol";

/** The supervisor proxies the daemon's rule list, so the browser never leaves its own origin. */
export const RULES_PATH = "/__rerail_playground/rules";
export const RULES_REFRESH_INTERVAL_MS = 5_000;
export const rulesQueryKey = ["playground", "rules"] as const;

export async function fetchRules(signal: AbortSignal): Promise<PlaygroundRuleSnapshot[]> {
  const response = await fetch(RULES_PATH, { signal, headers: { accept: "application/json" } });
  const body: unknown = await response.json().catch(() => undefined);
  if (!response.ok) {
    const detail = typeof body === "object" && body !== null && "error" in body ? String(body.error) : response.status;
    throw new Error(`Saved rules unavailable: ${detail}`);
  }
  return ruleListSchema.parse(body).rules;
}

/**
 * One line of rule shape for the header. The snapshot carries the saved document verbatim, so the
 * daemon's own schemas parse it; an unparseable definition is reported rather than rendered blank.
 */
export function describeRule(snapshot: PlaygroundRuleSnapshot | undefined): string {
  if (!snapshot) return "Waiting for the selected rule from the daemon.";
  let document: unknown;
  try {
    document = JSON.parse(snapshot.definition);
  } catch {
    return "Saved rule is not valid JSON; reload after fixing it in the daemon.";
  }
  if (snapshot.kind === "conversation") {
    const rule = conversationRuleSchema.safeParse(document);
    if (!rule.success) return "Saved cosine rule does not match the daemon schema.";
    return `${rule.data.sample.source} · ${rule.data.sample.boundary} · ${rule.data.mode}`;
  }
  const rule = codeRuleSchema.safeParse(document);
  if (!rule.success) return "Saved policy rule does not match the daemon schema.";
  return `Policy · ${rule.data.mode} · ${rule.data.clauses.length} checks · evaluated by the policy runtime`;
}

/** Pretty-print for the read-only rule panel; the definition is already the saved document. */
export function formatRuleJson(snapshot: PlaygroundRuleSnapshot | undefined): string {
  if (!snapshot) return "";
  try {
    return JSON.stringify(JSON.parse(snapshot.definition), null, 2);
  } catch {
    return snapshot.definition;
  }
}
