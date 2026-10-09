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
export {
  isRerailStarted,
  setCheckoutStarted,
  setRepositoryStarted,
  startStateReader,
  stopRerail,
  workspaceStartState,
} from "./repository-start.js";
export { RerailRuntime, type RerailRuntimeOptions } from "./runtime.js";
export { MATCH_ENTRY_TYPE, MUTATION_ENTRY_TYPE, TURN_ENTRY_TYPE } from "./session-state.js";
export {
  SESSION_EVENT_PAGE_LIMIT,
  type SessionEventRecord,
  SqliteSessionReader,
  SqliteSessionStore,
} from "./session-store.js";
