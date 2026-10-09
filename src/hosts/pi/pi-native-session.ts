import type { ExtensionContext } from "@earendil-works/pi-coding-agent";
import type { AdapterDelivery, AdapterEvent, AdapterSession } from "../../engine/adapter-contract.js";
import {
  createNativeSession,
  NativeSessionClient,
  type NativeSessionInitializationOptions,
  type NativeSessionRoute,
  type NativeSessionHost,
} from "../../harness/native-session-client.js";

export interface PiNativeSessionOptions {
  entries(context?: ExtensionContext): readonly unknown[];
  appendEntry(customType: string, data: unknown): void;
  deliver(client: NativeSessionClient, deliveries: readonly AdapterDelivery[]): void | Promise<void>;
  /** The extension's serialized initialize; a faulted client asks for it to rebuild the session. */
  reattach(context: ExtensionContext): Promise<unknown>;
  closeFallback?(): Promise<void>;
}

/** Owns Pi's daemon client without importing any host-local runtime or database code. */
export class PiNativeSession {
  readonly #options: PiNativeSessionOptions;
  #client: NativeSessionClient | undefined;
  #context: ExtensionContext | undefined;
  #session: AdapterSession | undefined;

  constructor(options: PiNativeSessionOptions) {
    this.#options = options;
  }

  current(): NativeSessionClient | undefined {
    return this.#client;
  }

  context(): ExtensionContext | undefined {
    return this.#context;
  }

  session(): AdapterSession | undefined {
    return this.#session;
  }

  rememberContext(context: ExtensionContext): void {
    this.#context = context;
  }

  clearContext(): void {
    this.#context = undefined;
  }

  async initialize(
    context: ExtensionContext,
    session: AdapterSession & { host: "pi" },
    rebuild: boolean,
    initialization: NativeSessionInitializationOptions,
  ): Promise<NativeSessionRoute> {
    const host = {
      current: () => this.#client,
      matchesCurrent: candidate => this.#session?.sessionId === candidate.sessionId && this.#session.cwd === candidate.cwd,
      close: async () => {
        await this.close();
        await this.#options.closeFallback?.();
      },
      activate: (client, activeContext, activeSession) => {
        this.#client = client;
        this.#context = activeContext;
        this.#session = activeSession;
      },
      entries: activeContext => this.#options.entries(activeContext ?? this.#context),
      appendEntry: this.#options.appendEntry,
      deliver: this.#options.deliver,
      reattach: this.#options.reattach,
    } satisfies NativeSessionHost<ExtensionContext>;
    return createNativeSession(host, context, session, rebuild, initialization);
  }

  async close(): Promise<void> {
    const closing = this.#client;
    this.#client = undefined;
    this.#session = undefined;
    await closing?.close();
  }

  async ingest(event: AdapterEvent): Promise<void> {
    const current = this.#client;
    if (current === undefined) throw new Error("cosine-rules extension received an event before session_start");
    await current.ingest(event);
  }
}
