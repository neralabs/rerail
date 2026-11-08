/**
 * The session runtime: one session's events through the engine, the resident engine service, the session and
 * presentation stores, repository start state, and the client control of the daemon. It imports only the engine.
 *
 * The public surface of this layer: what code outside `src/` imports from it (`scripts/check/check-boundaries.mjs`).
 */
export {
  DAEMON_STATE_FILE,
  type DaemonHealth,
  type DaemonLaunchOptions,
  type DaemonState,
  startDaemon,
  stopDaemon,
} from "./daemon-control.js";
export { type EngineService, ingestEngineEvent, startEngineService } from "./engine-service.js";
export {
  AdapterEventSchema,
  decodeRerailEventFrame,
  encodeRerailEventResult,
  RERAIL_EVENT_PROTOCOL_PREFACE,
} from "./event-protocol.js";
export { type HostRuntimePort, type HostRuntimeSessionState, RerailHostRuntime } from "./host-runtime.js";
export { OmpMessageContentSource, type OmpSessionEntryLike, OmpWatermarkStore } from "./omp-semantic.js";
export { SqlitePresentationPreferenceStore } from "./presentation-preference-store.js";
export { type AgentJudge, createAgentJudge, type JudgeOutcome, type JudgeTask } from "./merge-assist/agent-judge.js";
export {
  type CheckRunner,
  createLocalCheckRunner,
  decisionEntryFor,
  lastNumber,
  personDecisionEntry,
  type Resolution,
  type ResolutionEvidence,
  type ResolveInput,
  resolveConflict,
  type SideCheckOutcome,
  type SideEvidence,
} from "./merge-assist/resolver.js";
export {
  PANEL_PERSPECTIVES,
  PANEL_PROMPT_VERSION,
  panelAnswerSchema,
  panelJsonSchema,
  panelOrder,
  panelPerspective,
  panelPrompt,
  type PanelDesign,
  type PanelInput,
  type PanelResult,
  type PanelVote,
  runPanel,
} from "./merge-assist/panel.js";
export { deriveSignals, draftDecision, type DraftOutcome, validateSignals } from "./merge-assist/capture.js";
export {
  type CaptureInput,
  fenceData,
  MAX_RANKED,
  newJudgeNonce,
  type RankInput,
  rankJsonSchemaFor,
  rankPrompt,
  type RankVerdict,
  rankVerdictSchema,
  rankVerdictSchemaFor,
  type SignalsInput,
} from "./merge-assist/judge-prompts.js";
export {
  comparePositions,
  type ComparisonOutcome,
  type ConceptTree,
  contestPositions,
  extractPositions,
  MAX_AREA_UNITS,
  type OwnPosition,
  type PositionContest,
  type PositionsOutcome,
  type PositionVerdict,
  type RankedUnits,
  rankUnits,
  unitLine,
} from "./merge-assist/alignment.js";
export {
  createMergeAssistService,
  MERGE_ASSIST_ERROR_CODES,
  MergeAssistError,
  type MergeAssistActor,
  type MergeAssistErrorCode,
  type MergeAssistRuntime,
  type MergeAssistService,
  type MergeAssistServiceOptions,
  type MergeAssistSession,
} from "./merge-assist/service.js";
export {
  isRerailStarted,
  setCheckoutStarted,
  setRepositoryStarted,
  startStateReader,
  stopRerail,
  workspaceStartState,
} from "./repository-start.js";
export { RerailRuntime, type RerailRuntimeOptions } from "./runtime.js";
export { EVENTS_DROPPED_ENTRY_TYPE, MATCH_ENTRY_TYPE, MUTATION_ENTRY_TYPE, TURN_ENTRY_TYPE, type DroppedEvents } from "./session-state.js";
export {
  SESSION_EVENT_PAGE_LIMIT,
  type SessionEventRecord,
  SqliteSessionReader,
  SqliteSessionStore,
} from "./session-store.js";
export { type RepositorySetupIo, onRuleCount } from "./repository-start.js";
export { type NpmRunner } from "./npm.js";
