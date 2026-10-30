import type { CodeRule } from "../engine/code-rule/code-rule-schema.js";
import type { ConversationRule } from "../engine/catalog/rule-schema.js";
import { CODE_RULE_AST_LANGUAGES } from "../engine/code-rule/code-rule-languages.js";

import { loadRuleCatalog } from "../engine/catalog/rule-catalog.js";
import { openRerailDatabase } from "../engine/storage/database.js";
import { upsertLibraryRule } from "../engine/catalog/rule-library.js";
import { SqliteSessionStore, type WorkLink } from "../runtime/session-store.js";

/**
 * Throwaway mock rows so MCP and UI work can start before the rule generator
 * exists. Every rule here passes its kind's schema; `rerail seed` is idempotent.
 */
export const MOCK_CODE_RULES: readonly CodeRule[] = [
  {
    format: "rerail.policy/v1",
    id: "mock-no-console-log",
    revision: 1,
    mode: "steer",
    description: "Console logging leaks into production bundles; the structured logger carries context and levels.",
    scope: { include: ["web/src/**/*.ts", "web/src/**/*.tsx"], exclude: ["web/src/api/generated/**", "**/*.stories.tsx"] },
    clauses: [
      {
        id: "console-call",
        forbid: { regex: { source: "file.content", pattern: "\\bconsole\\.(log|debug)\\(" } },
        violation: { anchor: "witness.span", code: "RERAIL-MOCK-001", message: "Use the project logger instead of console." },
      },
    ],
  },
  {
    format: "rerail.policy/v1",
    id: "mock-lazy-query-shaping",
    revision: 2,
    mode: "steer",
    description: "Route reads must stay inside the lazy query plan; materializing before filtering defeats the ServerPlan budget.",
    scope: { include: ["api/app/routes/**/*.py"], exclude: ["api/app/routes/health.py"] },
    clauses: [
      {
        id: "list-of-query",
        forbid: { ast: { language: CODE_RULE_AST_LANGUAGES[0], rule: { pattern: "list($QUERY)" } } },
        violation: { anchor: "witness.span", code: "RERAIL-MOCK-002", message: "Query materialized before filters were applied." },
      },
    ],
  },
  {
    format: "rerail.policy/v1",
    id: "mock-generated-client-imports",
    revision: 1,
    mode: "log",
    description: "Frontend API calls go through the Hey API generated client so the OpenAPI schema stays the contract.",
    scope: { include: ["web/src/**/*.ts", "web/src/**/*.tsx"], exclude: ["web/src/api/generated/**"] },
    clauses: [
      {
        id: "raw-fetch",
        forbid: { regex: { source: "file.content", pattern: "fetch\\(\\s*[\"'`]/api/" } },
        violation: { anchor: "witness.span", code: "RERAIL-MOCK-003", message: "Call the API through the generated client." },
      },
      {
        id: "generated-client-rubric",
        llm: {
          instruction: "Decide whether this API call bypasses the generated client.",
          passConditions: ["The call uses a function imported from web/src/api/generated."],
          failConditions: ["The call is a hand-written fetch to an /api path."],
        },
        violation: { anchor: "witness.span", code: "RERAIL-MOCK-003", message: "This API call may bypass the generated client." },
      },
    ],
  },
];

export const MOCK_CONVERSATION_RULES: readonly ConversationRule[] = [
  {
    id: "mock-cosine-prefer-generated-client",
    mode: "steer",
    reference: "Call the backend through the generated API client instead of hand-written fetch calls.",
    match: { operator: "gte", threshold: 0.62 },
    sample: { source: "assistant-text", boundary: "sentence", minimumTokens: 6 },
    action: {
      content: "Reminder: use the generated client in web/src/api/generated rather than raw fetch.",
    },
    repeat: { mode: "after-gap", gapTurns: 3 },
  },
  {
    id: "mock-cosine-migration-warning",
    mode: "steer",
    reference: "Editing a database migration or altering a table schema.",
    match: { operator: "gte", threshold: 0.6 },
    sample: { source: "user-input", boundary: "message" },
    action: {
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
  codeRules: number;
  conversationRules: number;
  sessions: number;
  events: number;
  workLinks: number;
}

/** Stores the mock rules in the library (rules created by you); adding them to a repository is the caller's choice. */
export function seedRerailDatabase(path: string, options: SeedOptions = {}): SeedResult {
  const now = options.now ?? Date.now();
  const database = openRerailDatabase(path);
  try {
    const ruleCatalog = loadRuleCatalog();
    // Mock rows for development, shipped in this package: approved as written, so they run.
    for (const rule of MOCK_CODE_RULES) upsertLibraryRule(database, ruleCatalog, rule, "code", { now, approve: true });
    for (const rule of MOCK_CONVERSATION_RULES) upsertLibraryRule(database, ruleCatalog, rule, "conversation", { now, revision: 1, approve: true });
  } finally {
    database.$client.close();
  }

  const sessionStore = new SqliteSessionStore(path, { repositoryId: options.repositoryId });
  try {
    const sessionId = mockSessionId(options.repositoryId);
    const session = { host: "omp" as const, sessionId, cwd: options.cwd ?? process.cwd() };
    sessionStore.openSession(session, now - 60_000);
    const events = [
      { eventId: "mock-event-1", kind: "session-lifecycle", observedAt: now - 60_000, payload: { type: "session-initialize" } },
      { eventId: "mock-event-2", kind: "user-prompt", observedAt: now - 50_000, payload: { text: "Add a users page that calls /api/users." } },
      { eventId: "mock-event-3", kind: "call", observedAt: now - 40_000, payload: { toolName: "write", args: { path: "web/src/users.ts" } } },
      { eventId: "mock-event-4", kind: "turn-end", observedAt: now - 30_000, payload: { turn: 1 } },
    ];
    for (const event of events) sessionStore.append({ sessionId, ...event });
    sessionStore.closeSession(sessionId, now - 20_000);
    const workLinks = mockWorkLinks(sessionId);
    for (const link of workLinks) sessionStore.linkWork(link, now);
    return {
      codeRules: MOCK_CODE_RULES.length,
      conversationRules: MOCK_CONVERSATION_RULES.length,
      sessions: 1,
      events: events.length,
      workLinks: workLinks.length,
    };
  } finally {
    sessionStore.close();
  }
}
