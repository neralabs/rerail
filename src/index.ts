export {
  createConversationRulesExtension,
  deliverOmpGuidance,
  MATCH_ENTRY_TYPE,
  MUTATION_ENTRY_TYPE,
  TURN_ENTRY_TYPE,
  type ConversationRulesOptions,
} from "./hosts/omp/extension.js";
export { bundledModelPath } from "./engine/package-root.js";
export { createPiConversationRulesExtension, type PiConversationRulesOptions } from "./hosts/pi/pi-extension.js";
export {
  CustomerActivationManager,
  CustomerProfileStore,
  activationTrustRootsSchema,
  customerProfileSchema,
  flattenedJwsSchema,
  verifyCustomerProfile,
  type ActivationFetch,
  type ActivationRequest,
  type ActivationTrustRoots,
  type CustomerProfile,
  type DeviceIdentity,
  type FlattenedJws,
  type OfflineActivationRequestOptions,
  type OnlineActivationOptions,
  type SignedActivationRequest,
  type VerifiedCustomerProfile,
} from "./activation/customer-profile.js";
export { deviceSecretStore, FileDeviceSecretStore, SystemDeviceSecretStore, type DeviceSecretStore } from "./activation/device-keystore.js";
export {
  ADAPTER_CAPABILITIES,
  RULE_MESSAGE_TYPE,
  type AdapterCapabilityProfile,
  type AdapterDelivery,
  type AdapterEvent,
  type AdapterSample,
  type AdapterSession,
  type RerailEventLoop,
  type RerailHost,
  type SteerRule,
} from "./engine/adapter-contract.js";
export { claudeHookEvents, createClaudeEventHandler, type ClaudeHookInput } from "./hosts/claude/claude-event-adapter.js";
export { codexHookEvents, createCodexEventHandler, type CodexHookInput } from "./hosts/codex/codex-event-adapter.js";
export {
  decodeRerailEventFrame,
  encodeRerailEventResult,
  RERAIL_EVENT_MAX_FRAME_BYTES,
  RERAIL_EVENT_PROTOCOL_PREFACE,
  type RerailEventErrorFrame,
  type RerailEventFrame,
  type RerailEventResultFrame,
  type RerailEventSuccessFrame,
} from "./runtime/event-protocol.js";
export {
  runRerailEventStream,
  serveRerailEventStream,
  type RunRerailEventStreamOptions,
  type RerailEventStreamOptions,
} from "./harness/event-stream.js";
export { RerailHostRuntime, type HostRuntimePort, type HostRuntimeSessionState } from "./runtime/host-runtime.js";
export { CodeRuleBridge, generatedEditPaths } from "./engine/code-rule/code-rule-bridge.js";
export { NativeCodeRuleEnforcer } from "./engine/code-rule/code-rule-enforcer.js";
export {
  renderSteer,
  type CodeRuleEdit,
  type CodeRuleEditBatch,
  type CodeRuleAuditor,
  type CodeRuleViolation,
  type CodeRuleResponse,
  type AuditFailure,
  type CodeRuleDslAstPattern,
  type CodeRuleDslAstCount,
  type CodeRuleDslAstQuery,
  type CodeRuleDslAstRelation,
  type CodeRuleDslAstRule,
  type CodeRuleDslClause,
  type CodeRuleDslClauseBase,
  type CodeRuleDslCompare,
  type CodeRuleDslDocument,
  type CodeRuleDslEvidenceQuery,
  type CodeRuleDslRelation,
  type CodeRuleDslViolationAnchor,
  type CodeRuleDslViolationSpec,
  type CodeRuleDslPredicate,
  type CodeRuleDslQuery,
  type CodeRuleDslRegexQuery,
  type CodeRuleDslScalar,
  type CodeRuleDslScope,
  type CodeRuleDslSelectWhenQuery,
  type CodeRuleDslValue,
  type CodeRuleCheckpoint,
  type CodeRuleSourceLanguage,
  type CodeRuleViolationFix,
  type CodeRuleSourceSpan,
  type RuleSteerDelivery,
} from "./engine/code-rule/code-rule.js";
export type {
  CodeRuleDslSemanticAstRuleFields,
  CodeRuleDslSemanticCompareText,
  CodeRuleDslSuppress,
  CodeRuleDslArguments,
  CodeRuleDslBindingFilter,
  CodeRuleDslBinds,
  CodeRuleDslCalls,
  CodeRuleDslFileCondition,
  CodeRuleDslFormatArgs,
  CodeRuleDslLanguageOptions,
  CodeRuleDslParse,
  CodeRuleDslResolves,
  CodeRuleDslValueTest,
} from "./engine/code-rule/code-rule-semantic.js";
export {
  AUDIT_SCOPES,
  AUDIT_MODES,
  type AuditMode,
  type AuditRecord,
  type AuditSummary,
  type AuditScope,
} from "./engine/audit/audit.js";
export {
  SqliteAuditControlStore,
} from "./engine/audit/audit-control.js";
export {
  auditControlsSchema,
  auditSettingsPatchSchema,
  auditSettingsSchema,
  repositoryAuditSchema,
  auditRequestInputSchema,
  auditRequestSchema,
  type AuditControls,
  type AuditSettings,
  type RepositoryAudit,
  type AuditRequestInput,
  type AuditRequest,
} from "./engine/audit/audit-control-schema.js";
export {
  bundledCodeRuleDslDirectory,
  type CompiledCodeRuleDslEntry,
  type CodeRuleDslCatalog,
  type CodeRuleDslCatalogStatus,
} from "./engine/code-rule/code-rule-dsl-catalog.js";
export {
  SqliteSessionStore,
  SqliteSessionReader,
  decodeEventPayload,
  type SessionEvent,
  type SessionEventRecord,
  type StoredSessionSummary,
  type WorkLink,
  type WorkLinkRecord,
} from "./runtime/session-store.js";
export {
  SqliteAuditReader,
  SqliteAuditStore,
  type AuditReader,
  type AuditPage,
  type AuditQuery,
} from "./engine/audit/audit-store.js";
export {
  RULE_KINDS,
  RULE_ORIGINS,
  RULE_SCOPES,
  events,
  audits,
  ruleCatalogIdentities,
  rules,
  repositoryAudits,
  policySessionOwners,
  sessions,
  workLinks,
  auditSettings,
  auditRequests,
  type AuditRow,
  type EventRow,
  type NewRuleRow,
  type RuleCatalogIdentityRow,
  type RuleKind,
  type RuleOrigin,
  type RuleRow,
  type RuleScope,
  type SessionRow,
  type WorkLinkRow,
} from "./engine/storage/schema.js";
export { RERAIL_SCHEMA_SQL, RERAIL_SCHEMA_STATEMENTS } from "./engine/storage/schema-sql.generated.js";
export { RERAIL_SCHEMA_VERSION } from "./engine/storage/migrations.js";
export { defaultRerailDatabasePath, openRerailDatabase, type OpenRerailDatabaseOptions, type RerailDatabase } from "./engine/storage/database.js";
export {
  SqliteRuleStore,
  type RuleDocument,
  type RuleQuery,
  type RuleWriteOptions,
  type StoredConversationRule,
  type StoredCodeRule,
  type StoredRule,
} from "./engine/catalog/rule-store.js";
export {
  createCheckoutResolver,
  linkAlias,
  listRepositories,
  lookupCheckout,
  recordCheckout,
  type RepositoryListing,
} from "./engine/catalog/repository-registry.js";
export { addAttachment, listRepositoryRules, removeAttachment, setRuleSetting, setUpRepository } from "./engine/catalog/rule-configuration.js";
export { copyRerailRuleToLibrary, getLibraryRule, listLibraryRules, removeLibraryRule, upsertLibraryRule } from "./engine/catalog/rule-library.js";
export { approveRule, pendingRules, RuleApprovalError, type PendingRule, type RuleApproval } from "./engine/catalog/rule-approval.js";
export {
  CALLER_CWD_HEADER,
  folderContains,
  normalizeFolder,
  type ApiCaller,
  type RuleCatalog,
  type CheckoutContext,
  type CheckoutRecord,
  type CheckoutRuleSet,
  type EffectiveRule,
  type LibraryRule,
  type RepositoryKey,
  type RepositoryRecord,
  type RepositoryStartState,
  type RuleApprovalStatus,
  type RuleAttachment,
  type RuleSetting,
} from "./engine/catalog/repository-model.js";
export {
  normalizeRemoteAddress,
  readCloneId,
  readRemoteAddress,
  repositoryKeyOf,
  resolveCloneId,
  type RemoteAddress,
  type RepositoryIdentityLocation,
} from "./engine/catalog/repository-identity.js";
export { loadRuleCatalog } from "./engine/catalog/rule-catalog.js";
export { GitUnavailableError } from "./engine/git.js";
export {
  ApiError,
  invokeOperation,
  type Operation,
  type OperationDefinition,
  type OperationMethod,
  type RerailApiStores,
} from "./api/operation.js";
export {
  eventRecordSchema,
  operationInputShape,
  operations,
  workLinkInputSchema,
  workLinkRecordSchema,
  auditRecordSchema,
  auditQuerySchema,
} from "./api/operations.js";
export { DatabaseApiStores, type DatabaseApiStoresOptions } from "./api/stores.js";
export { rulesDashboardUrl, sessionSummarySchema, type RulesDashboardScope } from "./api/session-contracts.js";
export { libraryRuleRecordSchema, rerailRuleSchema } from "./api/repository-contracts.js";
export { createRerailApi } from "./api/http.js";
export { callerFromHeader, createRerailMcpHandler, createRerailMcpServer } from "./api/mcp.js";
export {
  MOCK_CONVERSATION_RULES,
  MOCK_CODE_RULES,
  MOCK_SESSION_ID,
  mockSessionId,
  mockWorkLinks,
  seedRerailDatabase,
  type SeedOptions,
  type SeedResult,
} from "./cli/seed.js";
export { ruleDocumentDigest, type Json } from "./engine/stable-json.js";
export { RerailRuntime, type RerailRuntimeOptions } from "./runtime/runtime.js";
export { setRepositoryStarted } from "./runtime/repository-start.js";
export { startDaemon, stopDaemon } from "./runtime/daemon-control.js";
export { RULE_MODES, steers, type RuleMode } from "./engine/catalog/rule-mode.js";
export { loadPackRuleDocuments } from "./engine/catalog/rule-documents.js";
export { codeRuleSchema, type CodeRule } from "./engine/code-rule/code-rule-schema.js";
export {
  CONVERSATION_RULE_DOCUMENT_VERSION,
  conversationRuleDocumentSchema,
  conversationRuleSchema,
  parseConversationRule,
  MATCH_OPERATORS,
  SAMPLE_BOUNDARIES,
  parseConversationRuleDocument,
  type ConversationRule,
  type ConversationRuleDocument,
  type MatchOperator,
  type SampleBoundary,
  type SampleSource,
} from "./engine/catalog/rule-schema.js";
export {
  ConversationRuleRegistry,
  type ConversationRuleRegistryOptions,
  type MatchOptions,
  type RuleInspection,
  type RuleMatch,
} from "./engine/catalog/registry.js";
export {
  loadNativeBinding,
  nativeHealth,
  type NativeBinding,
  type ModelFingerprint,
  type NativeCompiledCodeRuleDsl,
  type NativeHealth,
  type NativeCodeRuleDslRuntimeConstructor,
  type NativeCodeRuleDslEditor,
  type NativeSample as ConversationSample,
  type CodeRuleDslEvaluationMode,
  type CodeRuleDslSourceFormat,
} from "./engine/native/native.js";
export { InMemoryWatermarkStore, NativeSemanticEmbedder, SemanticWorkspace } from "./engine/native/semantic-primitives.js";
export {
  OmpMessageContentSource,
  OmpWatermarkStore,
  OMP_WATERMARK_ENTRY_TYPE,
  type OmpSessionEntryLike,
} from "./runtime/omp-semantic.js";
export type {
  ContentItem,
  ContentMetadata,
  ContentRef,
  ContentSnapshot,
  ContentSource,
  MarkReceipt,
  ProjectedVector,
  RelationReduction,
  ProjectionRender,
  ProjectionSpec,
  ProjectionUnit,
  RelationSpec,
  SelectedContent,
  Selection,
  SelectionSpec,
  SemanticEmbedder,
  SemanticPrimitives,
  SemanticProjection,
  SemanticRelationReduction,
  SemanticRelationSet,
  SemanticRelation,
  Watermark,
  WatermarkAssignment,
  WatermarkChange,
  WatermarkExpression,
  WatermarkSnapshot,
  WatermarkStore,
  WatermarkValue,
  WatermarkVisibility,
} from "./engine/native/semantic-types.js";
