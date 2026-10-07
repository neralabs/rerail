import type { PolicyRule } from "./policy-schema.js";
import type { CosineRule } from "./rule-schema.js";
import { POLICY_AST_LANGUAGES } from "./policy-languages.js";

import { SqliteRuleStore } from "./rule-store.js";
import { SqliteTranscriptStore, type WorkLink } from "./transcript-store.js";

/**
 * Throwaway mock rows so MCP and UI work can start before the rule generator
 * exists. Every rule here passes its kind's schema; `rerail seed` is idempotent.
 */
export const MOCK_POLICY_RULES: readonly PolicyRule[] = [
  {
    format: "rerail.policy/v1",
    id: "mock-no-console-log",
    revision: 1,
    description: "Console logging leaks into production bundles; the structured logger carries context and levels.",
    scope: { include: ["web/src/**/*.ts", "web/src/**/*.tsx"], exclude: ["web/src/api/generated/**", "**/*.stories.tsx"] },
    checks: [
      {
        id: "console-call",
        forbid: { regex: { source: "file.content", pattern: "\\bconsole\\.(log|debug)\\(" } },
        finding: { anchor: "witness.span", code: "RERAIL-MOCK-001", severity: "error", message: "Use the project logger instead of console." },
      },
    ],
  },
  {
    format: "rerail.policy/v1",
    id: "mock-lazy-query-shaping",
    revision: 2,
    description: "Route reads must stay inside the lazy query plan; materializing before filtering defeats the ServerPlan budget.",
    scope: { include: ["api/nera_api/routes/**/*.py"], exclude: ["api/nera_api/routes/health.py"] },
    checks: [
      {
        id: "list-of-query",
        forbid: { ast: { language: POLICY_AST_LANGUAGES[0], rule: { pattern: "list($QUERY)" } } },
        finding: { anchor: "witness.span", code: "RERAIL-MOCK-002", severity: "warning", message: "Query materialized before filters were applied." },
      },
    ],
  },
  {
    format: "rerail.policy/v1",
    id: "mock-generated-client-imports",
    revision: 1,
    description: "Frontend API calls go through the Hey API generated client so the OpenAPI schema stays the contract.",
    scope: { include: ["web/src/**/*.ts", "web/src/**/*.tsx"], exclude: ["web/src/api/generated/**"] },
    checks: [
      {
        id: "raw-fetch",
        forbid: { regex: { source: "file.content", pattern: "fetch\\(\\s*[\"'`]/api/" } },
        finding: { anchor: "witness.span", code: "RERAIL-MOCK-003", severity: "advisory", message: "Call the API through the generated client." },
      },
      {
        id: "generated-client-rubric",
        llm: {
          instruction: "Decide whether this API call bypasses the generated client.",
          passConditions: ["The call uses a function imported from web/src/api/generated."],
          failConditions: ["The call is a hand-written fetch to an /api path."],
        },
        finding: { anchor: "witness.span", code: "RERAIL-MOCK-003", severity: "advisory", message: "This API call may bypass the generated client." },
      },
    ],
  },
];

export const MOCK_COSINE_RULES: readonly CosineRule[] = [
  {
    id: "mock-cosine-prefer-generated-client",
    reference: "Call the backend through the generated API client instead of hand-written fetch calls.",
    match: { operator: "gte", threshold: 0.62 },
    sample: { source: "assistant-text", boundary: "sentence", minimumTokens: 6 },
    action: {
      delivery: "steer",
      content: "Reminder: use the generated client in web/src/api/generated rather than raw fetch.",
    },
    repeat: { mode: "after-gap", gapTurns: 3 },
  },
  {
    id: "mock-cosine-migration-warning",
    reference: "Editing a database migration or altering a table schema.",
    match: { operator: "gte", threshold: 0.6 },
    sample: { source: "user-input", boundary: "message" },
    action: {
      delivery: "nextTurn",
      content: "Schema changes need a migration note in the PR description.",
    },
    repeat: { mode: "once" },
  },
];

export const MOCK_SESSION_ID = "mock-session-0001";

/** The sample session of one repository; repositories share the database, so each needs its own id. */
export function mockSessionId(repositoryId?: string): string {
  return repositoryId === undefined ? MOCK_SESSION_ID : `mock-session-${repositoryId}`;
}

export function mockWorkLinks(sessionId: string): readonly WorkLink[] {
  return [
    { sessionId, kind: "git-branch", reference: "feature/rule-catalog" },
    { sessionId, kind: "linear-issue", reference: "ABC-1", url: "https://linear.app/example/issue/ABC-1" },
  ];
}

export interface SeedOptions {
  /** Directory recorded as the mock session's cwd. */
  cwd?: string;
  /** Repository the mock session is attributed to. */
  repositoryId?: string;
  now?: number;
}

export interface SeedResult {
  policyRules: number;
  cosineRules: number;
  sessions: number;
  events: number;
  workLinks: number;
}

export function seedRerailDatabase(path: string, options: SeedOptions = {}): SeedResult {
  const now = options.now ?? Date.now();
  const rules = new SqliteRuleStore(path);
  try {
    for (const rule of MOCK_POLICY_RULES) rules.upsertPolicy(rule, { now });
    for (const [index, rule] of MOCK_COSINE_RULES.entries()) {
      rules.upsertCosine(rule, { now, revision: 1, enabled: index === 0 });
    }
  } finally {
    rules.close();
  }

  const transcripts = new SqliteTranscriptStore(path, { repositoryId: options.repositoryId });
  try {
    const sessionId = mockSessionId(options.repositoryId);
    const session = { host: "omp" as const, sessionId, cwd: options.cwd ?? process.cwd() };
    transcripts.openSession(session, now - 60_000);
    const events = [
      { eventId: "mock-event-1", kind: "session-lifecycle", observedAt: now - 60_000, payload: { type: "session-initialize" } },
      { eventId: "mock-event-2", kind: "user-prompt", observedAt: now - 50_000, payload: { text: "Add a users page that calls /api/users." } },
      { eventId: "mock-event-3", kind: "tool-call", observedAt: now - 40_000, payload: { toolName: "write", args: { path: "web/src/users.ts" } } },
      { eventId: "mock-event-4", kind: "turn-end", observedAt: now - 30_000, payload: { turn: 1 } },
    ];
    for (const event of events) transcripts.append({ sessionId, ...event });
    transcripts.closeSession(sessionId, now - 20_000);
    const workLinks = mockWorkLinks(sessionId);
    for (const link of workLinks) transcripts.linkWork(link, now);
    return {
      policyRules: MOCK_POLICY_RULES.length,
      cosineRules: MOCK_COSINE_RULES.length,
      sessions: 1,
      events: events.length,
      workLinks: workLinks.length,
    };
  } finally {
    transcripts.close();
  }
}
