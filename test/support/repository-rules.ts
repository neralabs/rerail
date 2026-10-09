import {
  loadBundledCatalog,
  defaultRerailDatabasePath,
  openRerailDatabase,
  CheckoutCodeRuleCatalog,
  loadCodeRuleCapabilities,
  type CodeRule,
  type CheckoutRuleSet,
  type EffectiveRule,
  type RepositoryKey,
  recordCheckout,
  setUpRepository,
  upsertLibraryRule,
  type RuleDocument,
  ruleDocumentDigest,
  stableJsonText,
} from "../../src/engine/index.js";
import { setRepositoryStarted } from "../../src/runtime/index.js";

export interface TestRepositoryRules {
  /** Stored in the library and added to the repository. */
  readonly library?: readonly RuleDocument[];
  /** By Rerail rule ids added to the repository on their own. */
  readonly rerailRules?: readonly string[];
  readonly packs?: readonly string[];
  /** Where everything is added; the whole repository by default. */
  readonly folder?: string;
  /** Leave the repository's start state alone (a test that starts or stops it itself). */
  readonly start?: boolean;
}

/**
 * Rules run only where a repository adds them: record `workspace`'s checkout, add `rules` to its repository, mark it
 * set up and start it. Returns the repository key.
 */
export function setUpTestRepository(
  workspace: string,
  rules: TestRepositoryRules = {},
  databasePath: string = defaultRerailDatabasePath(),
): RepositoryKey {
  const database = openRerailDatabase(databasePath);
  try {
    const bundle = loadBundledCatalog();
    const { repositoryKey } = recordCheckout(database, workspace);
    const folder = rules.folder ?? "";
    for (const document of rules.library ?? []) upsertLibraryRule(database, bundle, document.rule, document.kind);
    setUpRepository(database, repositoryKey, {
      packs: (rules.packs ?? []).map(packId => ({ packId, folder })),
      rules: [
        ...(rules.library ?? []).map(document => ({ sourceKind: "library_rule" as const, sourceId: document.rule.id, folder })),
        ...(rules.rerailRules ?? []).map(sourceId => ({ sourceKind: "rerail_rule" as const, sourceId, folder })),
      ],
    });
    if (rules.start !== false) setRepositoryStarted(database, repositoryKey, true);
    return repositoryKey;
  } finally {
    database.$client.close();
  }
}

/**
 * A catalog of `codeRules`, on everywhere at their own mode, for tests that drive one enforcer directly. Pass a
 * function to let the set change between refreshes.
 */
export function testCodeRuleCatalog(
  codeRules: readonly CodeRule[] | (() => readonly CodeRule[]),
  capabilitiesPath?: string,
): CheckoutCodeRuleCatalog {
  const current = typeof codeRules === "function" ? codeRules : () => codeRules;
  const checkout = { checkoutPath: "/test-checkout", repositoryKey: "clone:test", cloneId: "test", git: false, main: true };
  const load = () => {
    const rules: EffectiveRule[] = current().map(rule => ({
      document: { kind: "file", rule },
      source: { kind: "library_rule", id: rule.id, packId: null },
      attachmentFolder: "",
      on: true,
      mode: rule.mode,
      decidedBy: { on: { by: "rule-default" }, mode: { by: "rule-default" } },
      revision: rule.revision,
      payloadHash: ruleDocumentDigest({ kind: "file", rule }),
    }));
    const ruleSet: CheckoutRuleSet = {
      checkout,
      status: { started: true, setUp: true, startedBy: "repository" },
      signature: stableJsonText(current()) ?? "",
      rulesFor: () => rules,
      conversationRules: () => [],
      unavailable: [],
    };
    return { ruleSet, folders: [""], quarantined: [] };
  };
  return new CheckoutCodeRuleCatalog(
    { label: "test", signature: () => stableJsonText(current()) ?? "", load },
    capabilitiesPath === undefined ? undefined : loadCodeRuleCapabilities(capabilitiesPath),
  );
}
