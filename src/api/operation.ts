/**
 * The shape every API operation shares, and the stores handlers read and write. One operation definition feeds
 * every consumer: the daemon's HTTP routes and OpenAPI document, the MCP tool list, and (through OpenAPI) the
 * generated web client. `params`, `query`, and `body` are split for HTTP; MCP tools take their union as one flat
 * argument object.
 */
import type { z } from "zod";
import type { RerailDatabase } from "../engine/storage/database.js";
import type { AuditReader } from "../engine/audit/audit-store.js";
import type { SqlitePresentationPreferenceStore } from "../runtime/presentation-preference-store.js";
import type { ApiCaller, BundledCatalog, CheckoutContext, RepositoryStartState } from "../engine/catalog/repository-model.js";
import type { CodingAgentState } from "../engine/adapter-contract.js";
import type { SqliteAuditControlStore } from "../engine/audit/audit-control.js";
import type { SqliteSessionReader, SqliteSessionStore } from "../runtime/session-store.js";
import type { presentationDaemonSchema } from "./session-contracts.js";
import { InvalidCursorError } from "../engine/storage/page-cursor.js";

export interface RerailApiStores {
  /** Read-only view; tolerates a database that does not exist yet. */
  readonly reader: SqliteSessionReader;
  /** Writable session/event/work-link store; opening it creates the database. */
  sessionStore(): SqliteSessionStore;
  /** Durable per-session audit defaults and explicit audit requests. */
  auditControls(): SqliteAuditControlStore;
  /** Read-only audit history; remains empty before any writer creates the database. */
  readonly audits: AuditReader;
  /** Daemon-owned presentation settings. */
  preferences(): SqlitePresentationPreferenceStore;
  /** Writable handle on the one database: repositories, checkouts, rule configuration and the library. */
  database(): RerailDatabase;
  /** The rules and packs that ship with Rerail. */
  bundle(): BundledCatalog;
  /** Passive checkout lookup (never records), with each workspace's git location cached. */
  checkouts(): (workspace: string) => CheckoutContext | undefined;
  /** Change token over the whole database: other connections' commits and writes through these stores. */
  dataVersion(): string;
  /** {@link dataVersion} once the database exists; undefined before, without creating it. */
  historyVersion(): string | undefined;
  /** Count a write made through these stores, for {@link dataVersion}. */
  markChanged(): void;
  /** Whether Rerail is on in the checkout a workspace is in, over one held connection with cached identities. */
  repositoryState(workspace: string): RepositoryStartState;
  /** `http://host:port` of the dashboard. */
  dashboardOrigin(): string;
  health(): z.infer<typeof presentationDaemonSchema>;
  /** Every coding agent and whether Rerail is connected to it; undefined when this process cannot check. */
  codingAgents(): Promise<CodingAgentState[]> | undefined;
}

export type OperationMethod = "GET" | "POST" | "PUT" | "PATCH" | "DELETE";

export interface OperationDefinition<
  Params extends z.ZodRawShape,
  Query extends z.ZodRawShape,
  Body extends z.ZodRawShape,
  Output extends z.ZodType,
> {
  /** MCP tool name and OpenAPI operationId: `[a-z0-9_]+`. */
  id: string;
  method: OperationMethod;
  /** OpenAPI-style path, e.g. `/api/library-rules/{id}`. */
  path: string;
  summary: string;
  params?: z.ZodObject<Params>;
  query?: z.ZodObject<Query>;
  body?: z.ZodObject<Body>;
  output: Output;
  handler: (
    stores: RerailApiStores,
    input: z.infer<z.ZodObject<Params>> & z.infer<z.ZodObject<Query>> & z.infer<z.ZodObject<Body>>,
    caller: ApiCaller,
  ) => z.infer<Output> | Promise<z.infer<Output>>;
}

export type Operation = OperationDefinition<z.ZodRawShape, z.ZodRawShape, z.ZodRawShape, z.ZodType>;

export function defineOperation<
  Params extends z.ZodRawShape,
  Query extends z.ZodRawShape,
  Body extends z.ZodRawShape,
  Output extends z.ZodType,
>(operation: OperationDefinition<Params, Query, Body, Output>): Operation {
  return operation as unknown as Operation;
}

/** Thrown by handlers; the HTTP adapter maps it to a status, the MCP adapter to an error result. */
export class ApiError extends Error {
  readonly status: number;
  readonly code: string;

  constructor(status: number, code: string, message: string) {
    super(message);
    this.name = "ApiError";
    this.status = status;
    this.code = code;
  }
}

/** Run one operation for one caller; a write that returns counts toward the stores' change token. */
export async function invokeOperation(operation: Operation, stores: RerailApiStores, input: Record<string, unknown>, caller: ApiCaller): Promise<unknown> {
  try {
    return await operation.handler(stores, input as never, caller);
  } catch (error) {
    if (error instanceof InvalidCursorError) throw new ApiError(400, "invalid-cursor", error.message);
    throw error;
  } finally {
    // A write that failed half way may still have committed; counting it costs one extra miss.
    if (operation.method !== "GET") stores.markChanged();
  }
}
