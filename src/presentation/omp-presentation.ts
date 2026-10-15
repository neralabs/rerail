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
}

/** Presentation follows the runtime's session lifecycle, never its enforcement path. */
export function createOmpPresentation(host: "omp" | "pi" = "omp"): OmpPresentation {
  let current: PresentationContext | undefined;
  let timer: unknown;
  let generation = 0;
  let pending = false;

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
      ctx.ui.setWidget(STATUS_KEY,
        presentation.presence === "silent" ? undefined : [formatTerminalStatus(presentation)],
        { placement: "belowEditor" });
    } catch {
      // The footer is one short line users stack with others, so the error's detail stays out of it; readPresentation
      // already turns every daemon failure into a state, so reaching here is a Rerail bug, not something to explain inline.
      if (!isCurrent()) return;
      ctx.ui.setWidget(STATUS_KEY, [formatTerminalStatus(fallbackPresentation("unavailable"))], { placement: "belowEditor" });
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
      ctx.ui.setWidget(STATUS_KEY, [formatTerminalStatus(notOnHerePresentation())], { placement: "belowEditor" });
      return;
    }
    const tick = (): void => {
      if (!pending) void refresh(ctx);
    };
    timer = ctx.setInterval === undefined ? setInterval(tick, REFRESH_MS).unref() : ctx.setInterval(tick, REFRESH_MS);
    // Daemon reads must not hold up session initialization or steer delivery.
    void refresh(ctx);
  }

  return { start, stop };
}
