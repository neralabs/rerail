import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import { afterEach, expect, it, vi } from "vitest";

import type * as RulesApi from "./rules-api";
import { useRerailPlayground } from "./useRerailPlayground";

vi.mock("./rules-api", async (importOriginal) => ({
  ...(await importOriginal<typeof RulesApi>()),
  fetchRules: vi.fn(async () => []),
}));
vi.mock("./usePlaygroundTerminal", () => {
  const terminal = { terminalHostRef: { current: null }, writeTerminal: vi.fn(), resetTerminal: vi.fn() };
  return { usePlaygroundTerminal: () => terminal };
});

class PlaygroundSocket {
  static OPEN = 1;
  static instances: PlaygroundSocket[] = [];
  readyState = 1;
  onopen: (() => void) | null = null;
  onmessage: ((event: MessageEvent<string>) => void) | null = null;
  onclose: ((event: CloseEvent) => void) | null = null;
  onerror: (() => void) | null = null;
  send = vi.fn();
  close = vi.fn();
  constructor() {
    PlaygroundSocket.instances.push(this);
  }
  receive(message: unknown) {
    this.onmessage?.(new MessageEvent("message", { data: JSON.stringify(message) }));
  }
}

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  PlaygroundSocket.instances = [];
});

it("keeps paid generation single-flight from the initial click until completion, failure, or Stop", async () => {
  vi.stubGlobal("WebSocket", PlaygroundSocket);
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } });
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );
  const { result, unmount } = renderHook(() => useRerailPlayground({ initialRuleId: "rule", onBack: vi.fn() }), {
    wrapper,
  });
  await waitFor(() => expect(PlaygroundSocket.instances).toHaveLength(1));
  const socket = PlaygroundSocket.instances[0]!;
  const rule = { id: "rule", kind: "conversation", definition: "{}", digest: "digest", updatedAt: "now", enabled: false };
  const scenario = {
    id: "scenario",
    title: "Positive",
    expectation: "match",
    explanation: "Relevant input",
    ruleDigest: "digest",
    ruleKind: "conversation",
    prompt: "Inspect the code",
    fixtures: [],
    steps: [],
  };
  const selection = { ruleId: "rule", scenarioId: "scenario", mode: "trigger", allowCommands: false };
  act(() => {
    socket.onopen?.();
    socket.receive({
      type: "experiment",
      rule,
      experiment: {
        ruleId: "rule",
        selection,
        scenarios: [scenario],
        lastRun: {
          rule,
          scenario,
          selection,
          completedAt: 1,
          trace: [],
          outcome: { observed: "match", detail: "Matched" },
        },
      },
    });
  });
  await waitFor(() => expect(result.current.canGenerate).toBe(true));
  socket.send.mockClear();
  act(() => {
    result.current.onGenerate();
    result.current.onGenerate();
    result.current.onRevise();
  });
  expect(socket.send.mock.calls.map(([text]) => JSON.parse(text))).toEqual([
    { type: "generate", ruleId: "rule", revise: false },
  ]);
  expect(result.current.status.state).toBe("generating");
  expect(result.current.canGenerate).toBe(false);
  act(() => socket.receive({ type: "status", state: "generating", message: "Generating scenarios" }));
  expect(result.current.canGenerate).toBe(false);
  expect(result.current.canRevise).toBe(false);
  expect(result.current.canRun).toBe(false);
  expect(result.current.canPause).toBe(false);
  expect(result.current.canStop).toBe(true);
  act(() => socket.receive({ type: "status", state: "stopped", message: "Scenarios generated" }));
  expect(result.current.canGenerate).toBe(true);
  act(() => result.current.onRevise());
  expect(result.current.canGenerate).toBe(false);
  act(() => socket.receive({ type: "status", state: "error", message: "Model unavailable" }));
  expect(result.current.canGenerate).toBe(true);
  expect(result.current.error).toBe("Model unavailable");
  act(() => result.current.onGenerate());
  expect(result.current.canGenerate).toBe(false);
  act(() => {
    result.current.onStop();
    socket.receive({ type: "status", state: "stopped", message: "Cancelled" });
  });
  expect(result.current.canGenerate).toBe(true);
  unmount();
  queryClient.clear();
});
