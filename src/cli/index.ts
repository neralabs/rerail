/**
 * The `rerail` command line. `cli.ts` is the executable and is not exported: importing it runs the CLI.
 *
 * The public surface of this layer: what code outside `src/` imports from it (`scripts/check/check-boundaries.mjs`).
 */
export {
  assertLocalEngineEntitlement,
  authorizedUpdateChannel,
  runProfileCommand,
  signedUpdatePolicy,
} from "./activation-cli.js";
export { runRulesCommand } from "./project-cli.js";
export {
  MOCK_CODE_RULES,
  MOCK_CONVERSATION_RULES,
  MOCK_SESSION_ID,
  mockSessionId,
  seedRerailDatabase,
} from "./seed.js";
export { runInstallCommand } from "./setup-cli.js";
export {
  SHOW_ME_FILE,
  SHOW_ME_FIXED,
  SHOW_ME_MESSAGE,
  SHOW_ME_RULE_ID,
  SHOW_ME_VIOLATING,
  runShowMeCommand,
} from "./show-me.js";
export { type DashboardPage, type SetupEnvironment, startRerail } from "./repository-setup.js";
export { type SetupContext, runSetupCommand, runSetupStepArgs, setupAnswerCommand } from "./setup-cli.js";
export { configureDataLayerAudit, prepareDataLayerAudit, runDataLayerAudit, runDataLayerCommand, type DataLayerAuditOptions, type DataLayerAuditResult } from "./data-layer-audit.js";
export { schemaApprovalMessage, type SchemaApprovals } from "./data-layer/approvals.js";
export { decodePythonBase64 as decodeSchemaApprovalSignature } from "./data-layer/base64.js";
