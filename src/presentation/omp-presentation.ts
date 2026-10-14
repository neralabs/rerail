import { fallbackPresentation, formatTerminalStatus, notOnHerePresentation, readPresentation } from "./session-presentation.js";

const STATUS_KEY = "rerail";
const REFRESH_MS = 5_000;

/**
 * What the footer needs from an OMP or Pi extension context. OMP schedules through its own timers so
 * they stop with the session; Pi has none, so the footer falls back to an unref'd process timer.
 */
export interface PresentationContext {
  cwd: string;
  hasUI?: boolean;
  sessionManager: { getSessionId(): string };
  ui: { setWidget(key: string, content: string[] | undefined, options?: { placement?: "aboveEditor" | "belowEditor" }): void };
  setInterval?(callback: () => void, ms: number): unknown;
  clearTimer?(timer: never): void;
}

export interface OmpPresentation {
  /** `off`: Rerail is off in this checkout, so nothing runs and no daemon is there to ask. */
  start(ctx: PresentationContext, options?: { off?: boolean }): void;
  stop(): void;
  /**
   * How many session events the host is dropping right now (its daemon is not answering, or the session is
   * failing to attach); 0 once it stops. The daemon's health cannot say so: `/healthz` may well answer.
   */
  dropped(count: number): void;
}

/** Presentation follows the runtime's session lifecycle, never its enforcement path. */
export function createOmpPresentation(host: "omp" | "pi" = "omp"): OmpPresentation {
  let current: PresentationContext | undefined;
  let timer: unknown;
  let generation = 0;
  let pending = false;
  /** The daemon's status lines last shown, beneath which the dropped-events line is painted. */
  let status: string[] = [];
  let droppedCount = 0;
  /** The person chose `silent`: no ambient line at all, the dropped-events one included. Its gap is still recorded. */
  let silent = false;

  function paint(ctx: PresentationContext): void {
    const droppedLine = `Rerail: ${droppedCount} session event${droppedCount === 1 ? "" : "s"} dropped — daemon not answering`;
    const lines = silent ? [] : droppedCount > 0 ? [...status, droppedLine] : status;
    ctx.ui.setWidget(STATUS_KEY, lines.length === 0 ? undefined : lines, { placement: "belowEditor" });
  }

  function show(ctx: PresentationContext, lines: string[], silenced = false): void {
    status = lines;
    silent = silenced;
    paint(ctx);
  }

  function stop(): void {
    generation += 1;
    if (current?.hasUI) {
      if (timer !== undefined) {
        if (current.clearTimer === undefined) clearInterval(timer as NodeJS.Timeout);
        else current.clearTimer(timer as never);
      }
      current.ui.setWidget(STATUS_KEY, undefined);
    }
    timer = undefined;
    current = undefined;
    pending = false;
    status = [];
    silent = false;
  }

  async function refresh(ctx: PresentationContext): Promise<void> {
    const sessionId = ctx.sessionManager.getSessionId();
    const cwd = ctx.cwd;
    const startedGeneration = generation;
    const isCurrent = () => current === ctx && generation === startedGeneration
      && ctx.sessionManager.getSessionId() === sessionId && ctx.cwd === cwd;
    pending = true;
    try {
      const presentation = await readPresentation({ sessionId, host, cwd });
      if (!isCurrent()) return;
      const silenced = presentation.presence === "silent";
      show(ctx, silenced ? [] : [formatTerminalStatus(presentation)], silenced);
    } catch {
      // The footer is one short line users stack with others, so the error's detail stays out of it; readPresentation
      // already turns every daemon failure into a state, so reaching here is a Rerail bug, not something to explain inline.
      if (!isCurrent()) return;
      show(ctx, [formatTerminalStatus(fallbackPresentation("unavailable"))]);
    } finally {
      if (generation === startedGeneration) pending = false;
    }
  }

  function start(ctx: PresentationContext, options: { off?: boolean } = {}): void {
    stop();
    if (!ctx.hasUI) return;
    current = ctx;
    if (options.off === true) {
      // Nothing autostarts where Rerail is off, so asking a daemon would only report it missing.
      show(ctx, [formatTerminalStatus(notOnHerePresentation())]);
      return;
    }
    const tick = (): void => {
      if (!pending) void refresh(ctx);
    };
    timer = ctx.setInterval === undefined ? setInterval(tick, REFRESH_MS).unref() : ctx.setInterval(tick, REFRESH_MS);
    // Daemon reads must not hold up session initialization or steer delivery.
    void refresh(ctx);
  }

  function dropped(count: number): void {
    if (count === droppedCount) return;
    droppedCount = count;
    if (current?.hasUI) paint(current);
  }

  return { start, stop, dropped };
}
