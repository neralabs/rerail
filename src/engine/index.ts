/**
 * The engine: the rule model and schemas, code-rule compile and enforce, the native bridge, the catalog,
 * rule packs and languages, and the database they persist to. It imports no other layer.
 *
 * The public surface of this layer: what code outside `src/` imports from it (`scripts/check/check-boundaries.mjs`).
 */
export * as schema from "./storage/schema.js";
export {
  ADAPTER_CAPABILITIES,
  type AdapterCapabilityProfile,
  type AdapterDelivery,
  type AdapterEvent,
  type AdapterSample,
  type AdapterSession,
  type RerailEventLoop,
  type RerailHost,
  RULE_MESSAGE_TYPE,
} from "./adapter-contract.js";
export { auditRequestSchema } from "./audit/audit-control-schema.js";
export { SqliteAuditControlStore } from "./audit/audit-control.js";
export { SqliteAuditReader, SqliteAuditStore } from "./audit/audit-store.js";
export { type AuditInput, type AuditRecorder, type AuditResult, isUnreadFailure, UNREAD_FAILURE_HEAD } from "./audit/audit.js";
export { loadBundledRules } from "./catalog/bundled-rule-documents.js";
export { loadBundledCatalog, readBundledCatalog } from "./catalog/bundled-rules.js";
export { ConversationRuleRegistry } from "./catalog/registry.js";
export { repositoryIdentityLocation, resolveCloneId } from "./catalog/repository-identity.js";
export {
  type BundledCatalog,
  CALLER_CWD_HEADER,
  type CheckoutContext,
  type CheckoutRuleSet,
  type EffectiveRule,
  type RepositoryKey,
  type RepositoryStartState,
  type RepositoryStatus,
} from "./catalog/repository-model.js";
export { checkoutStatus, listRepositories, lookupCheckout, recordCheckout } from "./catalog/repository-registry.js";
export { exportRulesDump, importRulesDump, type RulesDump } from "./catalog/rule-bootstrap.js";
export {
  addAttachment,
  listRepositoryRules,
  removeAttachment,
  ruleConfigurationSignature,
  setRuleSetting,
  setUpRepository,
} from "./catalog/rule-configuration.js";
export { getLibraryRule, listLibraryRules, upsertLibraryRule, writeLibraryRule } from "./catalog/rule-library.js";
export { RULE_MODES, type RuleMode } from "./catalog/rule-mode.js";
export {
  loadRulePacks,
  RULE_PACK_RULE_ID,
  type RulePack,
  type RulePackFile,
  rulePackFileSchema,
  rulePackPackageName,
  rulePackPackageSchema,
} from "./catalog/rule-packs.js";
export { type ConversationRule, parseConversationRule } from "./catalog/rule-schema.js";
export { type RuleDocument, SqliteRuleStore } from "./catalog/rule-store.js";
export { CodeRuleBridge } from "./code-rule/code-rule-bridge.js";
export {
  bundledCodeRuleDslDirectory,
  CheckoutCatalogs,
  CheckoutCodeRuleCatalog,
  loadCodeRuleCapabilities,
} from "./code-rule/code-rule-dsl-catalog.js";
export { IGNORED_DIRECTORIES, languageFor, NativeCodeRuleEnforcer } from "./code-rule/code-rule-enforcer.js";
export { languageNotSupportedOf } from "./code-rule/code-rule-language-support.js";
export {
  CODE_RULE_AST_LANGUAGES,
  CODE_RULE_DIALECTS,
  CODE_RULE_LANGUAGE_FAMILIES,
  codeRuleLanguageFamily,
  LANGUAGE_REGISTRY,
} from "./code-rule/code-rule-languages.js";
export { CODE_RULE_FORMAT, type CodeRule, codeRuleSchema, type CodeRuleScope } from "./code-rule/code-rule-schema.js";
export { type CodeRuleSourceLanguage, type CodeRuleViolation, type RuleSteerDelivery } from "./code-rule/code-rule.js";
export { headlineText, linkSteers, sessionStartNote, steerBlock, steerHeadlines } from "./code-rule/rerail-steer.js";
export { steerOrigin } from "./code-rule/steer-origin.js";
export { parseJsonWithoutDuplicateKeys } from "./json-duplicate-keys.js";
export {
  encodeCodeRuleWorkspaceRequest,
  loadNativeBinding,
  type NativeBinding,
  type NativeCodeRuleDslWorkspace,
  type NativeCompiledCodeRuleDsl,
  nativePlatformSuffix,
  resolveNativeBindingPath,
} from "./native/native.js";
export { NativeSemanticEmbedder, SemanticWorkspace } from "./native/semantic-primitives.js";
export { type SemanticEmbedder, type Watermark } from "./native/semantic-types.js";
export { compareDeterministicStrings, ruleDocumentDigest, stableJsonText } from "./stable-json.js";
export {
  defaultRerailDatabasePath,
  openRerailDatabase,
  openSqliteDatabase,
  type RerailDatabase,
  resetRepositoryHistory,
} from "./storage/database.js";
export { defaultRerailHome } from "./storage/rerail-home.js";
export { RERAIL_BASELINE_SCHEMA_VERSION, addColumn, type Migration } from "./storage/migrations.js";
export {
  auditRequests,
  auditResultPages,
  audits,
  auditSettings,
  checkouts,
  events,
  libraryRules,
  policySessionOwners,
  presentationPreferences,
  repositories,
  repositoryAudits,
  setupReviews,
  ruleAttachments,
  ruleCatalogIdentities,
  schemaMigrations,
  rules,
  ruleSettings,
  sessions,
  workLinks,
} from "./storage/schema.js";
export { ENTRY_POINTS, PACKAGE_ROOT } from "./package-root.js";
export { RERAIL_HOSTS, type CodingAgentState } from "./adapter-contract.js";
export { type ApiCaller } from "./catalog/repository-model.js";
