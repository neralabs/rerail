import type { ConversationRuleSource } from "./code-rule-dsl-catalog.js";
import type { ConversationRuleRegistry } from "./registry.js";
import type { ConversationRule } from "./rule-schema.js";
import { stableJsonText } from "./stable-json.js";

/**
 * Keeps a session's configured conversation rules synchronized without disturbing session-local
 * mutations or delivery repeat state. Publishes only successfully loaded identity.
 */
export class LiveConversationRules {
  readonly #source: ConversationRuleSource;
  readonly #registry: ConversationRuleRegistry;
  /** Every rule this sync put into the registry, by id, with its stable JSON. */
  readonly #managed = new Map<string, string>();
  sourceSignature = "";

  constructor(source: ConversationRuleSource, registry: ConversationRuleRegistry) {
    this.#source = source;
    this.#registry = registry;
  }

  /** The session-start load: replace the registry's rules with the configured set. */
  prime(): void {
    const sourceSignature = this.#source.signature();
    const rules = this.#source.rules();
    this.#registry.replace(rules);
    this.#managed.clear();
    for (const rule of rules) this.#managed.set(rule.id, stable(rule));
    this.sourceSignature = sourceSignature;
  }

  /** Apply what changed in the configuration since the last look. Returns whether the registry changed. */
  refresh(): boolean {
    const sourceSignature = this.#source.signature();
    if (sourceSignature === this.sourceSignature) return false;
    const rules = this.#source.rules();
    const next = new Map(rules.map(rule => [rule.id, stable(rule)] as const));
    let changed = false;
    for (const rule of rules) {
      if (this.#managed.get(rule.id) === next.get(rule.id)) continue;
      this.#registry.upsert(rule, { replace: true });
      changed = true;
    }
    const gone = [...this.#managed.keys()].filter(id => !next.has(id));
    if (gone.length > 0) {
      // Switched off, not forgotten: a `once` rule that already fired must not fire again when re-enabled.
      this.#registry.remove(gone, { forgetDelivery: false });
      changed = true;
    }
    this.#managed.clear();
    for (const [id, text] of next) this.#managed.set(id, text);
    this.sourceSignature = sourceSignature;
    return changed;
  }

  close(): void {
    this.#source.close();
  }
}

function stable(rule: ConversationRule): string {
  const text = stableJsonText(rule);
  if (text === undefined) throw new Error(`Cosine rule ${rule.id} is not JSON representable`);
  return text;
}
