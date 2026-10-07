import type { ExtensionContext } from "@oh-my-pi/pi-coding-agent";
import { formatTerminalStatus, readPresentation } from "./session-presentation.js";

const STATUS_KEY = "rerail";
const REFRESH_MS = 5_000;

export interface OmpPresentation {
  start(ctx: ExtensionContext): void;
  stop(): void;
}

/** Presentation follows the runtime's session lifecycle, never its enforcement path. */
export function createOmpPresentation(): OmpPresentation {
  let current: ExtensionContext | undefined;
  let timer: NodeJS.Timeout | undefined;
  let generation = 0;
  let pending = false;

  function stop(): void {
    generation += 1;
    if (current?.hasUI) {
      if (timer !== undefined) current.clearTimer(timer);
      current.ui.setWidget(STATUS_KEY, undefined);
    }
    timer = undefined;
    current = undefined;
    pending = false;
  }

  async function refresh(ctx: ExtensionContext): Promise<void> {
    const sessionId = ctx.sessionManager.getSessionId();
    const cwd = ctx.cwd;
    const startedGeneration = generation;
    const isCurrent = () => current === ctx && generation === startedGeneration
      && ctx.sessionManager.getSessionId() === sessionId && ctx.cwd === cwd;
    pending = true;
    try {
      const presentation = await readPresentation({ sessionId, host: "omp", cwd });
      if (!isCurrent()) return;
      ctx.ui.setWidget(STATUS_KEY,
        presentation.presence === "silent" ? undefined : [formatTerminalStatus(presentation)],
        { placement: "belowEditor" });
    } catch (error) {
      if (!isCurrent()) return;
      const message = String(error instanceof Error ? error.message : error)
        .replace(/[\p{Cc}\p{Cf}\p{Zl}\p{Zp}]/gu, "");
      ctx.ui.setWidget(STATUS_KEY, [`Rerail unavailable: ${message}`], { placement: "belowEditor" });
    } finally {
      if (generation === startedGeneration) pending = false;
    }
  }

  function start(ctx: ExtensionContext): void {
    stop();
    if (!ctx.hasUI) return;
    current = ctx;
    timer = ctx.setInterval(() => {
      if (!pending) void refresh(ctx);
    }, REFRESH_MS);
    // Daemon reads must not hold up session initialization or policy delivery.
    void refresh(ctx);
  }

  return { start, stop };
}
