import { createHash, randomBytes } from "node:crypto";
import { zodToOpenAPIRegistry, type ZodOpenAPIMetadata } from "@asteasolutions/zod-to-openapi";
import { createRoute, OpenAPIHono, z } from "@hono/zod-openapi";
import type { Context, MiddlewareHandler } from "hono";
import { duplicateJsonKeyError } from "../json-duplicate-keys.js";
import {
  codeRuleAstRelationSchema,
  codeRuleAstRuleSchema,
  codeRuleFormatArgsSchema,
  codeRulePredicateSchema,
  codeRuleOptionConditionSchema,
  codeRuleParseSchema,
  codeRuleQuerySchema,
  codeRuleSchema,
  codeRuleUtilitySchema,
  codeRuleValueSchema,
} from "../code-rule-schema.js";
import { semanticSchemaComponents } from "../code-rule-schema-semantic.js";

import { conversationRuleSchema, matchConditionSchema, sampleSourceSchema } from "../rule-schema.js";
import { auditControlsSchema, auditRequestSchema } from "../audit-control-schema.js";
import { ApiError, invokeOperation, type Operation, type RerailApiStores } from "./operation.js";
import { eventRecordSchema, auditRecordSchema, presentationPreferenceSchema } from "./operations.js";
import { deciderSchema, effectiveRuleSchema, libraryRuleRecordSchema, repositorySummarySchema } from "./repository-contracts.js";
import { sessionResponseSchema, sessionSummarySchema } from "./session-contracts.js";

// Named components give the generated client stable type names, and break the recursive
// cycles (ast rules, parsed literals, format arguments, evidence predicates, select_when queries) with a $ref instead of infinite expansion.
// Registered directly rather than via `.openapi()`: zod 4 copies prototype methods onto instances
// at construction, so schemas built before the OpenAPI extension loaded lack that method.
const COMPONENTS: ReadonlyArray<[z.ZodType, string]> = [
  [codeRuleValueSchema, "PolicyValue"],
  [codeRuleAstRuleSchema, "PolicyAstRule"],
  [codeRuleUtilitySchema, "PolicyUtility"],
  [codeRuleOptionConditionSchema, "PolicyOptionCondition"],
  [codeRuleAstRelationSchema, "PolicyAstRelation"],
  ...semanticSchemaComponents(codeRuleParseSchema, codeRuleFormatArgsSchema),
  [codeRulePredicateSchema, "PolicyPredicate"],
  [codeRuleQuerySchema, "PolicyQuery"],
  [codeRuleSchema, "FileRule"],
  [matchConditionSchema, "MatchCondition"],
  [conversationRuleSchema, "ConversationRule"],
  [effectiveRuleSchema, "EffectiveRule"],
  [deciderSchema, "RuleDecider"],
  [libraryRuleRecordSchema, "LibraryRule"],
  [repositorySummarySchema, "RepositorySummary"],
  [sessionResponseSchema, "SessionResponse"],
  [eventRecordSchema, "EventRecord"],
  [auditRecordSchema, "AuditRecord"],
  [presentationPreferenceSchema, "PresentationPreference"],
  [auditControlsSchema, "AuditControls"],
  [auditRequestSchema, "AuditRequest"],
];
for (const [schema, refId] of COMPONENTS) zodToOpenAPIRegistry.add(schema, { _internal: { refId } });

// Preserve recursive nulls and template patterns that the OpenAPI converter loses.
const valueRegistry = z.registry<{ id: string }>();
valueRegistry.add(codeRuleValueSchema, { id: "PolicyValue" });
const valueSchemas = z.toJSONSchema(valueRegistry, { uri: id => `#/components/schemas/${id}` });
zodToOpenAPIRegistry.add(codeRuleValueSchema, {
  _internal: { refId: "PolicyValue" },
  // The converter's shared metadata type excludes JSON Schema null; this document is OpenAPI 3.1.
  anyOf: valueSchemas.schemas.PolicyValue!.anyOf as NonNullable<ZodOpenAPIMetadata<unknown>["anyOf"]>,
});
zodToOpenAPIRegistry.add(sampleSourceSchema, {
  type: "string",
  pattern: z.toJSONSchema(z.templateLiteral([sampleSourceSchema])).pattern!,
});

const errorSchema = z.object({ error: z.string(), message: z.string() }).openapi("ApiError");
const invalidRequestSchema = z.object({ error: z.literal("invalid-request"), issues: z.array(z.unknown()) }).openapi("InvalidRequest");

const jsonContent = (schema: z.ZodType, description: string) => ({
  description,
  content: { "application/json": { schema } },
});

/**
 * GET operations answered entirely from the database, so its change token dates them without running the
 * handler. Conditional requests are an HTTP concern, so they live here rather than in the shared operation
 * shape that MCP tools also read.
 */
const DATABASE_ETAG_OPERATIONS: ReadonlySet<string> = new Set([
  "repositories_list", "repository_rules_get", "library_rules_list", "library_rules_get", "rerail_rules_list",
]);

/**
 * GET operations that read audit history, which has no cheap change token: their tag is a
 * digest of the result, so the handler runs and only the response body is saved.
 */
const RESULT_ETAG_OPERATIONS: ReadonlySet<string> = new Set(["rules_activity", "violations_list", "sessions_list"]);

const ETAG_OPERATIONS: ReadonlySet<string> = new Set([...DATABASE_ETAG_OPERATIONS, ...RESULT_ETAG_OPERATIONS]);

/** Key order in a query string must not change the tag; two filters that differ must. */
function normalizedInput(input: Record<string, unknown>): string {
  return JSON.stringify(Object.entries(input).sort(([left], [right]) => (left < right ? -1 : 1)));
}

/** Strong entity-tag over whatever dates the response, scoped to the operation that produced it. */
function entityTag(operationId: string, ...parts: readonly string[]): string {
  const digest = createHash("sha256").update([operationId, ...parts].join("\u0000"));
  return `"${digest.digest("hex").slice(0, 32)}"`;
}

/** `If-None-Match` carries a list; a strong comparison is an exact match against one member. */
function matchesEtag(header: string | undefined, etag: string): boolean {
  if (header === undefined) return false;
  return header.split(",").some(candidate => candidate.trim() === etag);
}

/**
 * Hostnames a browser reaches a local daemon by. A DNS-rebinding page resolves its own name to
 * 127.0.0.1, so the socket cannot tell it apart; only the `Host` header (the name the browser
 * typed) can. The port is deliberately not part of this: behind a container port mapping or a
 * reverse proxy the page is addressed to a port the daemon never bound.
 */
const LOOPBACK_HOSTNAMES: ReadonlySet<string> = new Set(["127.0.0.1", "localhost", "[::1]"]);

/** Hostnames besides loopback that a deployment answers to; empty unless `--public-host` names some. */
export type PublicHosts = ReadonlySet<string>;

/** `Host` grammar: a bracketed IPv6 literal or a name, then an optional decimal port. `::1:4737` is malformed. */
const HOST_PATTERN = /^(\[[0-9a-f:.]+\]|[^\s:[\]/?#@]+)(?::(\d{1,5}))?$/iu;

/** Lower-cased hostname (IPv6 keeps its brackets) and port of a `Host`-style `name[:port]`; `undefined` when malformed. */
function parseHost(host: string): { name: string; port: string | undefined } | undefined {
  const match = HOST_PATTERN.exec(host.trim());
  if (match === null) return undefined;
  return { name: match[1]!.toLowerCase(), port: match[2] };
}

/** Whether a request addressed to `host` may reach the daemon at all. Missing or malformed is a no. */
export function allowedHost(host: string | undefined, publicHosts: PublicHosts): boolean {
  if (host === undefined) return false;
  const parsed = parseHost(host);
  return parsed !== undefined && (LOOPBACK_HOSTNAMES.has(parsed.name) || publicHosts.has(parsed.name));
}

/** Command-line `--public-host` values normalised the way `Host` is compared: lower-case, no port. */
export function publicHostSet(hosts: Iterable<string>): PublicHosts {
  const names = new Set<string>();
  for (const host of hosts) {
    const parsed = parseHost(host);
    if (parsed === undefined || parsed.port !== undefined) throw new Error(`Invalid public host: ${host} (a hostname without a port)`);
    names.add(parsed.name);
  }
  return names;
}

/**
 * 421 for every request whose `Host` is not one the daemon is known by, before any route runs
 * (RFC 9110 §15.5.20: the server is not able to produce a response for that authority). Mounted
 * first so the private bridges, the MCP endpoint, health, and the page are all behind it.
 */
export function hostAllowlist(publicHosts: PublicHosts): MiddlewareHandler {
  return async (context, next) => {
    if (!allowedHost(context.req.header("host"), publicHosts)) return context.json({ error: "host-not-allowed" }, 421);
    await next();
  };
}

/**
 * Whether a browser-reported `Origin` is a page this daemon serves: an allowlisted hostname on the
 * port the request was addressed to. The request's `Host` supplies the port rather than the
 * daemon's bound one, because behind a container port mapping or a reverse proxy both headers
 * come from the same browser and describe the same page. `Host` may omit a default port that
 * `URL` already strips from `origin.host`, so both sides are normalised the same way.
 */
function sameOrigin(origin: string, host: string | undefined, publicHosts: PublicHosts): boolean {
  if (host === undefined) return false;
  let parsed: URL;
  try {
    parsed = new URL(origin);
  } catch {
    return false;
  }
  if (parsed.protocol !== "http:" && parsed.protocol !== "https:") return false;
  const originName = parsed.hostname.toLowerCase();
  if (!LOOPBACK_HOSTNAMES.has(originName) && !publicHosts.has(originName)) return false;
  const requested = parseHost(host);
  if (requested === undefined) return false;
  const defaultPort = parsed.protocol === "http:" ? "80" : "443";
  return (parsed.port === "" ? defaultPort : parsed.port) === (requested.port ?? defaultPort);
}

/**
 * Mount every operation as an HTTP route and publish the OpenAPI document that
 * the generated web client is built from.
 */
export function createRerailApi(
  stores: RerailApiStores,
  operations: readonly Operation[],
  info: { version: string; publicHosts?: PublicHosts },
): OpenAPIHono {
  // `PRAGMA data_version` counts from zero on each new SQLite connection, so a catalog tag minted
  // before a restart could otherwise match one minted after.
  const instanceSalt = randomBytes(8).toString("hex");
  const publicHosts = info.publicHosts ?? new Set<string>();
  const app = new OpenAPIHono({
    defaultHook: (result, context) =>
      result.success ? undefined : context.json({ error: "invalid-request" as const, issues: result.error.issues }, 400),
  });

  // Browsers always send Origin on a cross-origin write; Rerail's own processes send none at all.
  // A served page is same-origin with the API, so nothing else has a legitimate reason to write.
  app.use("/api/*", async (context, next) => {
    const origin = context.req.header("origin");
    if (context.req.method !== "GET" && origin !== undefined && !sameOrigin(origin, context.req.header("host"), publicHosts)) {
      return context.json({ error: "api-origin-forbidden" }, 403);
    }
    await next();
  });

  // The framework parses bodies with JSON.parse, which keeps the last of repeated keys; check the raw text first.
  app.use("/api/*", async (context, next) => {
    if (context.req.method !== "GET" && context.req.method !== "HEAD" && (context.req.header("content-type") ?? "").includes("json")) {
      const duplicate = duplicateJsonKeyError(await context.req.text(), "request body");
      if (duplicate !== undefined) return context.json({ error: "duplicate-key", message: duplicate }, 400);
    }
    await next();
  });

  for (const operation of operations) {
    const route = createRoute({
      operationId: operation.id,
      method: operation.method.toLowerCase() as Lowercase<Operation["method"]>,
      path: operation.path,
      summary: operation.summary,
      request: {
        ...(operation.params === undefined ? {} : { params: operation.params }),
        ...(operation.query === undefined ? {} : { query: operation.query }),
        ...(operation.body === undefined
          ? {}
          : { body: { required: true, content: { "application/json": { schema: operation.body } } } }),
      },
      responses: {
        200: jsonContent(operation.output, "Success"),
        ...(ETAG_OPERATIONS.has(operation.id)
          ? { 304: { description: "Unchanged since the tag in If-None-Match; the response has no body." } }
          : {}),
        400: jsonContent(z.union([invalidRequestSchema, errorSchema]), "Invalid request"),
        403: jsonContent(errorSchema, "Not permitted, e.g. a coding agent changing a set-up repository's rules"),
        404: jsonContent(errorSchema, "Not found"),
        409: jsonContent(errorSchema, "Conflict"),
        503: jsonContent(errorSchema, "A dependency the operation needs, such as the native kernel, is unavailable"),
      },
    });
    app.openapi(route, async (context: Context) => {
      const validated = context.req as unknown as { valid(target: "param" | "query" | "json"): Record<string, unknown> | undefined };
      const input = { ...validated.valid("param"), ...validated.valid("query"), ...validated.valid("json") };
      const conditional = (etag: string): Response | undefined => {
        context.header("etag", etag);
        return matchesEtag(context.req.header("if-none-match"), etag) ? context.body(null, 304) : undefined;
      };
      try {
        if (DATABASE_ETAG_OPERATIONS.has(operation.id)) {
          // `dataVersion()` is a `PRAGMA data_version` read plus a local write counter, so it
          // catches commits from other processes as well as this daemon's own, at constant cost.
          const unchanged = conditional(entityTag(operation.id, instanceSalt, normalizedInput(input), stores.dataVersion()));
          if (unchanged !== undefined) return unchanged;
        }
        // `/api` callers are people (the dashboard, the CLI); coding agents come through `/mcp`.
        const result = await invokeOperation(operation, stores, input, { kind: "person" });
        if (RESULT_ETAG_OPERATIONS.has(operation.id)) {
          const unchanged = conditional(entityTag(operation.id, JSON.stringify(result) ?? ""));
          if (unchanged !== undefined) return unchanged;
        }
        return context.json(result, 200);
      } catch (error) {
        if (error instanceof ApiError) return context.json({ error: error.code, message: error.message }, error.status as 400);
        throw error;
      }
    });
  }

  app.doc31("/openapi.json", {
    openapi: "3.1.0",
    info: { title: "Rerail", version: info.version, description: "Rules, sessions, events, and work links for one project database." },
  });
  return app;
}
