import type { CosineRuleRegistry } from "./registry.js";
import type { CosineRule } from "./rule-schema.js";
import type { DatabaseCosineRules } from "./rule-sources.js";
import type { SqliteRuleStore } from "./rule-store.js";
import type { ScopedRuleCatalog } from "./scoped-rule-catalog.js";
import { stableJsonText } from "./stable-json.js";

/** Merge the database's enabled cosine rows with whatever else the host supplies (JSON files, options). */
export type CosineRuleLoader = (database: DatabaseCosineRules) => readonly CosineRule[];


/**
 * Keeps sourced cosine rules synchronized without disturbing session-local
 * mutations or delivery repeat state. Publishes only successfully loaded identity.
 */
export class LiveCosineRules {
  readonly #store: SqliteRuleStore | ScopedRuleCatalog;
  readonly #registry: CosineRuleRegistry;
  readonly #load: CosineRuleLoader;
  /** Every rule this sync put into the registry, by id, with its stable JSON. */
  readonly #managed = new Map<string, string>();
  sourceSignature = "";

  constructor(store: SqliteRuleStore | ScopedRuleCatalog, registry: CosineRuleRegistry, load: CosineRuleLoader) {
    this.#store = store;
    this.#registry = registry;
    this.#load = load;
  }

  /** The session-start load: replace the registry's rules with the merged set. */
  prime(): void {
    const sourceSignature = "ruleSignature" in this.#store ? this.#store.ruleSignature("conversation") : String(this.#store.dataVersion());
    const database = this.#read();
    const merged = this.#load(database);
    this.#registry.replace(merged);
    this.#managed.clear();
    for (const rule of merged) this.#managed.set(rule.id, stable(rule));
    this.sourceSignature = sourceSignature;
  }

  /** Apply what changed in the database since the last look. Returns whether the registry changed. */
  refresh(): boolean {
    const sourceSignature = "ruleSignature" in this.#store ? this.#store.ruleSignature("conversation") : String(this.#store.dataVersion());
    if (sourceSignature === this.sourceSignature) return false;
    const database = this.#read();

    const merged = this.#load(database);
    const next = new Map(merged.map(rule => [rule.id, stable(rule)] as const));
    let changed = false;
    for (const rule of merged) {
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
    this.#store.close();
  }

  #read(): DatabaseCosineRules {
    const rules = this.#store.list({ kind: "conversation" }).flatMap(stored => stored.kind === "conversation" ? [stored] : []);
    return { enabled: rules.filter(stored => stored.enabled).map(stored => stored.rule), ids: new Set(rules.map(stored => stored.rule.id)) };
  }
}

function stable(rule: CosineRule): string {
  const text = stableJsonText(rule);
  if (text === undefined) throw new Error(`Cosine rule ${rule.id} is not JSON representable`);
  return text;
}
