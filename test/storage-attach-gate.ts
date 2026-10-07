import { vi } from "vitest";
import * as native from "../src/native.js";
import type { NativeBinding, NativePolicyDslWorkspace } from "../src/native.js";

export interface GatedWorkspace {
  /** The real native workspace behind the proxy the code under test holds. */
  readonly native: NativePolicyDslWorkspace;
  /** Lets this workspace's storage attach start. */
  release(): void;
  /** Settles once the real attach has settled, either way. */
  readonly attached: Promise<void>;
}

export interface StorageAttachGate {
  /** Whether workspaces created from now on wait for `release()` before attaching storage. */
  gated: boolean;
  /** Every workspace created since the gate was installed, in creation order. */
  readonly workspaces: GatedWorkspace[];
  restore(): void;
}

/** Real native policy workspaces whose storage attach starts only when the test releases it. */
export function gateStorageAttachments(): StorageAttachGate {
  const binding = native.loadNativeBinding();
  const workspaces: GatedWorkspace[] = [];
  const gate: StorageAttachGate = { gated: true, workspaces, restore: () => spy.mockRestore() };
  class Workspace {
    constructor() {
      const inner = new binding.PolicyDslWorkspace();
      const start = Promise.withResolvers<void>();
      const attached = Promise.withResolvers<void>();
      if (!gate.gated) start.resolve();
      workspaces.push({ native: inner, release: () => start.resolve(), attached: attached.promise });
      return new Proxy(inner, {
        get(target, key) {
          if (key === "enableStorageAsync")
            return async (root: string, storePath: string) => {
              await start.promise;
              try {
                return await target.enableStorageAsync(root, storePath);
              } finally {
                attached.resolve();
              }
            };
          const value: unknown = Reflect.get(target, key, target);
          return typeof value === "function" ? value.bind(target) : value;
        },
      });
    }
  }
  const spy = vi
    .spyOn(native, "loadNativeBinding")
    .mockReturnValue(Object.assign(Object.create(binding) as NativeBinding, { PolicyDslWorkspace: Workspace }));
  return gate;
}
