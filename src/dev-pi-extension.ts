import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import type { ExtensionAPI, ExtensionContext, ExtensionFactory } from "@earendil-works/pi-coding-agent";
import { sendNativeDeliveries, type AdapterEvent, type AdapterSession } from "./adapter-contract.js";
import { isRerailDisabled } from "./checkout-control.js";
import { PiDevNativeSession } from "./pi-dev-native-session.js";
import { registerPiHostAdapter } from "./pi-host-adapter.js";

interface InitializeOptions {
  rebuild?: boolean;
  /** Only (re)attach while the context is still the host's current one; a stale caller becomes a no-op. */
  onlyIfCurrent?: boolean;
}

/** Pi runtime owner for an explicitly selected development container; it never opens a host-local runtime. */
export function createPiDevExtension(): ExtensionFactory {
  return function piDevExtension(pi: ExtensionAPI): void {
    let attaching: Promise<void> | undefined;
    const native = new PiDevNativeSession({
      entries: context => context?.sessionManager.getBranch() ?? [],
      appendEntry: (customType, data) => pi.appendEntry(customType, data),
      deliver: (current, deliveries) => sendNativeDeliveries(
        deliveries,
        (message, options) => pi.sendMessage(message, options),
        (sessionId, receiptId, ruleIds) => current.recordSteeringDelivery(sessionId, receiptId, ruleIds),
      ),
      // Serialized through `attaching` and dropped once its context stopped being current, so a
      // probe that lost the race to a session switch cannot resurrect the old session.
      reattach: context => initialize(context, { onlyIfCurrent: true }),
    });

    const closeRuntimeNow = (): Promise<void> => native.close();
    const closeRuntime = (): Promise<void> => {
      const pending = (attaching ?? Promise.resolve()).then(closeRuntimeNow, closeRuntimeNow).finally(() => {
        if (attaching === pending) attaching = undefined;
      });
      attaching = pending;
      return pending;
    };
    const initializeNow = async (context: ExtensionContext, options: InitializeOptions): Promise<void> => {
      if (options.onlyIfCurrent === true && native.context() !== context) return;
      if (isRerailDisabled(context.cwd)) {
        await closeRuntimeNow();
        native.rememberContext(context);
        return;
      }
      const session = {
        host: "pi",
        sessionId: context.sessionManager.getSessionId(),
        cwd: context.cwd,
      } satisfies AdapterSession & { host: "pi" };
      const activation = await native.initialize(context, session, options.rebuild === true, {});
      if (activation === "disabled") {
        native.rememberContext(context);
        return;
      }
      if (activation !== "attached") {
        throw new Error("Rerail development instance is unavailable; the development bridge cannot use a host-local fallback");
      }
    };
    const initialize = (context: ExtensionContext, options: InitializeOptions = {}): Promise<void> => {
      const start = () => initializeNow(context, options);
      const pending = (attaching ?? Promise.resolve()).then(start, start).finally(() => {
        if (attaching === pending) attaching = undefined;
      });
      attaching = pending;
      return pending;
    };
    const sessionFor = async (): Promise<AdapterSession | undefined> => {
      while (attaching !== undefined) await attaching;
      return native.session();
    };
    const ingest = (event: AdapterEvent): Promise<void> => native.ingest(event);

    registerPiHostAdapter(pi, {
      activeSession: () => native.session(),
      currentContext: () => native.context(),
      owner: () => native.current(),
      semantic: () => {
        const current = native.current();
        if (current === undefined) throw new Error("rerail semantic tools received an event before session_start");
        return current;
      },
      initialize: (context, rebuild) => initialize(context, { rebuild }),
      close: closeRuntime,
      shutdown: async () => {
        try {
          await closeRuntime();
        } finally {
          native.clearContext();
        }
      },
      sessionFor,
      ingest,
      cosineTool: {
        remoteExecute: () => {
          const current = native.current();
          return current === undefined ? undefined : input => current.cosine(input);
        },
      },
      cosineCommand: {
        remoteCommand: () => {
          const current = native.current();
          return current === undefined ? undefined : args => current.cosine({ action: "command", args });
        },
      },
      packageRoot: resolve(dirname(fileURLToPath(import.meta.url)), ".."),
    });
  };
}

export default createPiDevExtension();
