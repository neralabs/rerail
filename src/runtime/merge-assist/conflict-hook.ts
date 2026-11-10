/**
 * The conflict hook (docs/merge-assist-v1-build.md §12.5): an extensible point after detection. Each detection pass
 * fires `detected` (first seen in this checkout), `level-changed` (its level rose) and `closed` (dismissed, resolved, or
 * no longer found) once per conflict, and every registered handler hears each one in the background: the pass, the
 * session's boundary and the agent never wait for a handler. A handler that throws is logged and skipped.
 *
 * Built-in handlers are always registered: delivery, summary (the quick call) and the person's command. Experimental
 * handlers are registered only while their flag is on (`experimental-features.ts`): verdicts and chat answers. They
 * take part through {@link ConflictExperiment}: the shipped path reaches verdict and chat-answer code only through it.
 */
import type { RerailHost } from "../../engine/adapter-contract.js";
import type { CheckoutContext } from "../../engine/catalog/repository-model.js";
import type { ChatAnswer } from "../../engine/merge-assist/chat-answer.js";
import { CONFLICT_LEVELS, type ConflictDelivery, type ConflictEvent, type ConflictHookOutcome, type ConflictSummary, type MergeAssistSettings } from "../../engine/merge-assist/model.js";
import type { RerailDatabase } from "../../engine/storage/database.js";
import type { AgentJudge } from "./agent-judge.js";
import type { StoredConflict } from "./conflict-store.js";

/** What every handler hears an event with. */
export interface ConflictHandlerContext {
  readonly checkout: { readonly path: string; readonly repositoryKey: string; readonly branch: string | null; readonly base: string | null };
  /** The session whose boundary ran the pass, with its host; null for the dashboard, the CLI and the API. */
  readonly session: { readonly id: string; readonly host: RerailHost | null } | null;
  readonly delivery: ConflictDelivery;
  readonly settings: MergeAssistSettings;
  /** The person's agent CLI, under the repository's hourly budget. */
  readonly judge: AgentJudge;
  /** The person's environment, which the command runs with. */
  readonly env: NodeJS.ProcessEnv;
  readonly now: () => number;
  /** Offer the checkout's running sessions what is pending for them now, at their next boundary. */
  readonly deliver: () => void;
  /** The conflicts these events first detected, newest first. */
  readonly detected: readonly string[];
}

/** What a handler records on the conflict: the status line's summary, or the command's answer. */
export interface HandlerResult {
  readonly summary?: ConflictSummary;
  readonly hook?: ConflictHookOutcome;
}

export interface ConflictHandler {
  readonly name: string;
  on(event: ConflictEvent, conflict: StoredConflict, context: ConflictHandlerContext): Promise<HandlerResult | void>;
  /** Whether the conflict's agent note waits for this handler's answer to `event`, so what it adds joins the note. */
  awaits?(event: ConflictEvent, context: ConflictHandlerContext): boolean;
  /** The stages an experimental handler takes part in beyond events. */
  readonly experiment?: ConflictExperiment;
}

/** One conflict event of a pass, with the conflict as the pass left it (as it was, for one no longer found). */
export interface ConflictFire {
  readonly event: ConflictEvent;
  readonly conflict: StoredConflict;
}

/** A person's verdict on an open conflict. */
export interface VerdictInput {
  readonly database: RerailDatabase;
  readonly checkout: CheckoutContext;
  readonly now: () => number;
  readonly stored: StoredConflict;
  readonly action: "mine" | "theirs" | "new";
  readonly text: string | undefined;
  readonly name: string;
}

/** The person's prompt in a session, which may answer a card that session was handed. */
export interface ChatInput {
  readonly database: RerailDatabase;
  readonly checkout: CheckoutContext;
  readonly now: () => number;
  readonly prompt: string;
  readonly sessionId: string;
  /** Record a verdict as the person's, as the dashboard does. */
  readonly resolve: (conflictId: string, action: "mine" | "theirs" | "new", text: string | undefined) => Promise<void>;
}

/** The stages an experimental handler can take part in. */
export interface ConflictExperiment {
  /** Settle an open conflict with a person's verdict; the caller stores what it answers. */
  resolve?(input: VerdictInput): Promise<StoredConflict>;
  /** Read the person's prompt as an answer to a card; `none` when it is not one. */
  answer?(input: ChatInput): Promise<ChatAnswer>;
  /** The card offers the verdicts (keep mine, take theirs, new decision, later) instead of talking to the author. */
  readonly verdictCards?: true;
}

/** The event between a conflict as the store held it (or did not) and as it holds it now; null when nothing happened. */
function eventOf(was: StoredConflict | undefined, entry: StoredConflict): ConflictEvent | null {
  if (was === undefined) return entry.conflict.status === "open" ? "detected" : null;
  if (was.conflict.status !== "open") return null;
  if (entry.conflict.status !== "open") return "closed";
  return CONFLICT_LEVELS.indexOf(entry.conflict.level) > CONFLICT_LEVELS.indexOf(was.conflict.level) ? "level-changed" : null;
}

/**
 * The events between what the store held before a change and after it: `detected` for an open conflict it did not
 * hold, `level-changed` for an open one whose level rose, `closed` for an open one now settled or gone.
 */
export function conflictEvents(before: readonly StoredConflict[], after: readonly StoredConflict[]): ConflictFire[] {
  const prior = new Map(before.map(entry => [entry.conflict.id, entry]));
  const now = new Set(after.map(entry => entry.conflict.id));
  const changed = after.flatMap(entry => {
    const event = eventOf(prior.get(entry.conflict.id), entry);
    return event === null ? [] : [{ event, conflict: entry }];
  });
  const gone = before.filter(entry => entry.conflict.status === "open" && !now.has(entry.conflict.id));
  return [...changed, ...gone.map(entry => ({ event: "closed" as const, conflict: entry }))];
}

const failure = (error: unknown): string => (error instanceof Error ? error.message : String(error));

/**
 * The most a conflict's agent note waits for a handler that `awaits` its event: past it, the note goes out without
 * that handler's addition, which follows on its own (`notes.ts`, the hook's late note).
 */
export const NOTE_WAIT_MS = 1_000;

/**
 * The registered handlers, and the events they are working on. Handlers run in this process's background; a
 * conflict's agent note waits while a handler that `awaits` the event runs, at most {@link NOTE_WAIT_MS}.
 */
export class ConflictHook {
  readonly #handlers: readonly ConflictHandler[];
  readonly #noteWaitMs: number;
  /** Conflicts whose agent note waits for a handler, by checkout, with how many handlers it waits for. */
  readonly #awaiting = new Map<string, Map<string, number>>();
  /** Who waits for a checkout's notes to wait for nothing, by checkout. */
  readonly #released = new Map<string, Array<() => void>>();
  readonly #running = new Set<Promise<void>>();

  constructor(handlers: readonly ConflictHandler[], noteWaitMs = NOTE_WAIT_MS) {
    this.#handlers = handlers;
    this.#noteWaitMs = noteWaitMs;
  }

  /** The handlers' names, in the order they hear an event. */
  get names(): string[] {
    return this.#handlers.map(handler => handler.name);
  }

  /** The experiments registered, in order. */
  get experiments(): ConflictExperiment[] {
    return this.#handlers.flatMap(handler => (handler.experiment === undefined ? [] : [handler.experiment]));
  }

  /**
   * Hand each event to every handler, in the background, and record what each answers with `apply`. Returns at once:
   * a conflict's agent note waits only for the handlers that `awaits` its event, as {@link awaiting} says, and for each
   * at most the note wait. Every wait is registered before any handler starts, so a handler that hands notes out at
   * once (delivery) never hands out a note another handler (the command) is still to add to.
   */
  fire(fires: readonly ConflictFire[], context: ConflictHandlerContext, apply: (conflictId: string, result: HandlerResult) => void): void {
    const jobs = fires.flatMap(({ event, conflict }) => this.#handlers.map(handler => ({
      event, conflict, handler, release: this.#awaits(handler, event, context) ? this.#hold(context, conflict.conflict.id) : undefined,
    })));
    for (const job of jobs) {
      const work = this.#run(job, context, apply);
      this.#running.add(work);
      void work.finally(() => this.#running.delete(work));
    }
  }

  /** Make the conflict's note wait for one handler, until it answers or the note wait passes; returns the release. */
  #hold(context: ConflictHandlerContext, conflictId: string): () => void {
    this.#wait(context.checkout.path, conflictId, 1);
    let released = false;
    const release = (): void => {
      if (released) return;
      released = true;
      clearTimeout(timer);
      this.#wait(context.checkout.path, conflictId, -1);
      context.deliver();
    };
    const timer = setTimeout(release, this.#noteWaitMs);
    timer.unref();
    return release;
  }

  /** One handler on one event: its answer recorded and offered, a failure logged, its wait released. */
  async #run(
    { event, conflict, handler, release }: ConflictFire & { handler: ConflictHandler; release: (() => void) | undefined },
    context: ConflictHandlerContext,
    apply: (conflictId: string, result: HandlerResult) => void,
  ): Promise<void> {
    try {
      const result = await handler.on(event, conflict, context);
      if (result !== undefined && (result.summary !== undefined || result.hook !== undefined)) {
        apply(conflict.conflict.id, result);
        context.deliver();
      }
    } catch (error) {
      process.stderr.write(`rerail: Merge Assist's conflict handler ${handler.name} failed on ${event} of ${conflict.conflict.id}: ${failure(error)}\n`);
    } finally {
      release?.();
    }
  }

  /** Resolves once no conflict's agent note in the checkout waits for a handler: at most the note wait from now. */
  released(checkoutPath: string): Promise<void> {
    if (!this.#awaiting.has(checkoutPath)) return Promise.resolve();
    const { promise, resolve } = Promise.withResolvers<void>();
    this.#released.set(checkoutPath, [...(this.#released.get(checkoutPath) ?? []), resolve]);
    return promise;
  }

  /** The conflicts in the checkout whose agent note waits for a handler now. */
  awaiting(checkoutPath: string): ReadonlySet<string> {
    return new Set(this.#awaiting.get(checkoutPath)?.keys() ?? []);
  }

  /** Resolves once every handler running now has finished, and those they started. */
  async settled(): Promise<void> {
    while (this.#running.size > 0) await Promise.allSettled([...this.#running]);
  }

  #awaits(handler: ConflictHandler, event: ConflictEvent, context: ConflictHandlerContext): boolean {
    try {
      return handler.awaits?.(event, context) ?? false;
    } catch (error) {
      process.stderr.write(`rerail: Merge Assist's conflict handler ${handler.name} failed on ${event}: ${failure(error)}\n`);
      return false;
    }
  }

  #wait(checkoutPath: string, conflictId: string, change: 1 | -1): void {
    const waiting = this.#awaiting.get(checkoutPath) ?? new Map<string, number>();
    const count = (waiting.get(conflictId) ?? 0) + change;
    if (count > 0) waiting.set(conflictId, count);
    else waiting.delete(conflictId);
    if (waiting.size > 0) {
      this.#awaiting.set(checkoutPath, waiting);
      return;
    }
    this.#awaiting.delete(checkoutPath);
    for (const resolve of this.#released.get(checkoutPath) ?? []) resolve();
    this.#released.delete(checkoutPath);
  }
}
