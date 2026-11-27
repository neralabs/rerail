import type { AdapterEvent } from "../engine/adapter-contract.js";
import { EVENTS_DROPPED_ENTRY_TYPE, type DroppedEvents } from "../runtime/session-state.js";
import { countDroppedEvent } from "./native-session-client.js";

/** Attaching starts or resolves a daemon, so retries begin at a second and settle at half a minute. */
const ATTACH_RETRY_MS = 1_000;
const ATTACH_RETRY_CEILING_MS = 30_000;
function attachBackoffMs(failures: number): number {
  return Math.min(ATTACH_RETRY_MS * 2 ** Math.max(failures - 1, 0), ATTACH_RETRY_CEILING_MS);
}

/** What an OMP or Pi extension gives the retry of its session's attach. */
export interface AttachRetryHost<Context> {
  /** The extension's serialized attach of `context`'s session, its runtime built again when `rebuild` says; a failure rejects. */
  initialize(context: Context, rebuild: boolean): Promise<void>;
  sessionId(context: Context): string;
  appendEntry(customType: string, data: unknown): void;
  /** Shows how many events the failing session has dropped; 0 clears it. */
  reportDropped(count: number): void;
  /** Told of each failed try of the current attach, so the footer can follow the failing session. */
  failed(context: Context): void;
  /** The runtime attached to `sessionId` now, which records that session's gap; undefined when none is. */
  recorder(sessionId: string): { recordDropped(gaps: readonly DroppedEvents[]): void } | undefined;
}

/**
 * A session that failed to attach (daemon down or wedged at startup) surfaces that failure to the host exactly once, then
 * keeps trying quietly in the background until Rerail is back. Each host-driven attach opens a generation; a retry whose
 * build was in flight when a newer session (or shutdown) arrived belongs to a stale generation and must not reschedule
 * itself. What arrives for the failing session meanwhile is counted as dropped, and the runtime that attaches records the
 * count as a gap in the session's history.
 */
export class AttachRetry<Context> {
  readonly #host: AttachRetryHost<Context>;
  #timer: ReturnType<typeof setTimeout> | undefined;
  #generation = 0;
  #failures = 0;
  /** The context whose attach failed and is being retried; undefined once one attaches. */
  #failing: Context | undefined;
  /** What arrived for `#failing`'s session meanwhile: the attached runtime records it as a gap. */
  #drops: { sessionId: string; gap: DroppedEvents } | undefined;
  /**
   * The context of the host's newest attach, which its reattach attaches, whether that attach succeeded, failed or still
   * runs; none once the host shuts down.
   */
  #requested: Context | undefined;

  constructor(host: AttachRetryHost<Context>) {
    this.#host = host;
  }

  /** The context whose attach failed and is being retried, if any. */
  failing(): Context | undefined {
    return this.#failing;
  }

  /** The host's attach of `context`: a new generation, tried again after each failure until one attaches. */
  attach(context: Context, rebuild = false): Promise<void> {
    clearTimeout(this.#timer);
    this.#failures = 0;
    this.#requested = context;
    return this.#attachAs(context, ++this.#generation, rebuild);
  }

  async #attachAs(context: Context, generation: number, rebuild: boolean): Promise<void> {
    try {
      await this.#host.initialize(context, rebuild);
    } catch (error) {
      if (generation === this.#generation) this.#failed(context, generation, rebuild);
      throw error;
    }
    if (generation === this.#generation) this.#attached(context);
  }

  #failed(context: Context, generation: number, rebuild: boolean): void {
    this.#failures += 1;
    this.#failing = context;
    // Another session's drops keep their marker in that session, which got their final count before the host left
    // it, but the footer counts only this one's.
    if (this.#drops !== undefined && this.#drops.sessionId !== this.#host.sessionId(context)) {
      this.#drops = undefined;
      this.#host.reportDropped(0);
    }
    this.#host.failed(context);
    this.#timer = setTimeout(() => { this.#attachAs(context, generation, rebuild).catch(() => undefined); }, attachBackoffMs(this.#failures));
    this.#timer.unref();
  }

  #attached(context: Context): void {
    if (this.#failures > 0) process.stderr.write(`Rerail attached after ${this.#failures} failed attempt(s)\n`);
    this.#failures = 0;
    this.#failing = undefined;
    const dropped = this.#drops;
    this.#drops = undefined;
    if (dropped === undefined) return;
    this.#host.reportDropped(0);
    // Events of a session the host left have nowhere to be recorded but their marker, which got its final count before
    // the host left that session.
    if (dropped.sessionId !== this.#host.sessionId(context)) return;
    this.#host.appendEntry(EVENTS_DROPPED_ENTRY_TYPE, { ...dropped.gap, recovered: this.#recorded(dropped) });
  }

  /**
   * Whether the runtime attached to the dropped events' session recorded their gap. With no runtime (Rerail was stopped
   * meanwhile) or a record that failed, the session's marker is all that keeps the count; a failure is said on stderr.
   */
  #recorded(dropped: { sessionId: string; gap: DroppedEvents }): boolean {
    const recorder = this.#host.recorder(dropped.sessionId);
    if (recorder === undefined) return false;
    try {
      recorder.recordDropped([dropped.gap]);
      return true;
    } catch (error) {
      const reason = error instanceof Error ? error.message : String(error);
      process.stderr.write(`Rerail could not record ${dropped.gap.count} dropped session event(s); it is not retried: ${reason}\n`);
      return false;
    }
  }

  /**
   * Counts the event `build` makes for the session failing to attach, if there is one; the attached runtime records it.
   * `build` stays pure: nothing counted here changes what a later event of the same message carries.
   */
  countDrop(build: (sessionId: string) => AdapterEvent): boolean {
    if (this.#failing === undefined) return false;
    const sessionId = this.#host.sessionId(this.#failing);
    const event = build(sessionId);
    const previous = this.#drops?.sessionId === sessionId ? this.#drops.gap : undefined;
    const gap = countDroppedEvent(previous, event.kind, event.eventId);
    this.#drops = { sessionId, gap };
    if (previous === undefined) this.#host.appendEntry(EVENTS_DROPPED_ENTRY_TYPE, { ...gap, recovered: false });
    this.#host.reportDropped(gap.count);
    return true;
  }

  /**
   * The gap of a session that never attached is recorded only by its host-session marker, so before the host leaves that
   * session the marker gets the final count. The host appends to whichever session is current, so this runs while it is
   * still `context`'s; a switch that is then cancelled keeps counting, and a later marker carries the higher count.
   */
  markFinal(context: Context): void {
    const drops = this.#drops;
    if (drops === undefined || drops.sessionId !== this.#host.sessionId(context)) return;
    this.#host.appendEntry(EVENTS_DROPPED_ENTRY_TYPE, { ...drops.gap, recovered: false });
  }

  /**
   * No try is made again, as for a host that detaches from Rerail; what was dropped stays for the session's marker and for
   * the host's reattach.
   */
  cancel(): void {
    clearTimeout(this.#timer);
    this.#generation += 1;
    this.#failing = undefined;
  }

  /**
   * The host's reattach (after Rerail's start or stop, or a removal that failed), rebuilt: of the session of the host's
   * newest attach, which neither host holds a context of while that attach fails or runs (a failed start never activated
   * one; a failed or running switch holds the session left), else of `held`, the host's. One that fails is tried again;
   * one that attaches records what was dropped.
   */
  resume(held: Context | undefined): Promise<void> {
    const context = this.#requested ?? held;
    return context === undefined ? Promise.resolve() : this.attach(context, true);
  }

  /**
   * The host is shutting down in `context`: no try is made again, the failing session's marker gets its final count, and
   * the newest attach is forgotten, so a reattach after it rebuilds only a session the host still holds.
   */
  stop(context: Context): void {
    this.cancel();
    this.#requested = undefined;
    this.markFinal(context);
    this.#drops = undefined;
    this.#host.reportDropped(0);
  }
}
