import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useMemo, useRef, useState, type RefObject } from "react";

import type {
  PlaygroundControl,
  PlaygroundEvent,
  PlaygroundExperiment,
  PlaygroundOutcome,
  PlaygroundRuleSnapshot,
  PlaygroundScenario,
  PlaygroundSelection,
  PlaygroundTrace,
} from "@protocol";
import { eventSchema, scenarioSchema } from "@protocol";
import {
  describeRule,
  fetchRules,
  formatRuleJson,
  RULES_REFRESH_INTERVAL_MS,
  rulesQueryKey,
} from "./rules-api";

import { usePlaygroundTerminal } from "./usePlaygroundTerminal";

type RunStatus = Extract<PlaygroundEvent, { type: "status" }>;
type ExperimentEvent = Extract<PlaygroundEvent, { type: "experiment" }>;
type TraceRow = PlaygroundTrace & { id: number };
type Connection = "connecting" | "connected" | "reconnecting" | "disconnected";

const EXPERIMENT_KEY = ["rerail", "playground", "experiment"] as const;
const TRACE_LIMIT = 250;
const inputEncoder = new TextEncoder();

type PlaygroundCapabilities = Pick<
  RerailPlaygroundState,
  "canRun" | "canGenerate" | "canRevise" | "canSaveScenario" | "canPause" | "canStep" | "canStop"
>;
type CapabilityInputs = {
  connection: Connection;
  status: RunStatus;
  selection: PlaygroundSelection | undefined;
  scenario: PlaygroundScenario | undefined;
  scenarioDirty: boolean;
  hasLastRun: boolean;
};
const deriveCapabilities = (input: CapabilityInputs): PlaygroundCapabilities => {
  const connected = input.connection === "connected";
  const state = input.status.state;
  const busy = state === "starting" || state === "generating" || state === "rebuilding";
  const active = state === "running" || state === "paused";
  const scripted = input.selection?.mode !== "agent";
  return {
    canRun: connected && input.selection !== undefined && input.scenario !== undefined && !input.scenarioDirty && !busy,
    canGenerate: connected && input.selection !== undefined && !busy,
    canRevise: connected && input.selection !== undefined && input.hasLastRun && !busy,
    canSaveScenario: connected && input.scenario !== undefined && input.scenarioDirty && !busy,
    canPause: connected && scripted && active,
    canStep: connected && scripted && state === "paused",
    // A completed run can still have a DB watch armed; Stop remains available until disconnected.
    canStop: connected,
  };
};

const parsePlaygroundEvent = (data: unknown): PlaygroundEvent | undefined => {
  let raw: unknown;
  try {
    raw = typeof data === "string" ? JSON.parse(data) : null;
  } catch {
    raw = null;
  }
  const parsed = eventSchema.safeParse(raw);
  return parsed.success ? parsed.data : undefined;
};

/** The saved run whose trace replaces the current one: only when new to this connection or from a different run. */
const traceReplacementRun = (
  message: ExperimentEvent,
  previous: ExperimentEvent | undefined,
  restored: boolean,
): NonNullable<PlaygroundExperiment["lastRun"]> | undefined => {
  const lastRun = message.experiment.lastRun;
  if (!lastRun) return undefined;
  const replace =
    !restored ||
    previous?.experiment.ruleId !== message.experiment.ruleId ||
    previous.experiment.lastRun?.completedAt !== lastRun.completedAt;
  return replace ? lastRun : undefined;
};

export type RerailPlaygroundOptions = {
  initialRuleId?: string;
  onBack: () => void;
};

export type RerailPlaygroundState = {
  selection: PlaygroundSelection | undefined;
  experiment: PlaygroundExperiment | undefined;
  ruleSnapshot: PlaygroundRuleSnapshot | undefined;
  rules: PlaygroundRuleSnapshot[];
  selectedRule: PlaygroundRuleSnapshot | undefined;
  selectedRuleId: string;
  ruleJson: string;
  ruleSummary: string;
  scenario: PlaygroundScenario | undefined;
  scenarioJson: string;
  scenarioDirty: boolean;
  scenarioStale: boolean;
  connection: Connection;
  status: RunStatus;
  error: string | null;
  trace: TraceRow[];
  liveOutcome: PlaygroundOutcome | undefined;
  terminalHostRef: RefObject<HTMLElement | null>;
  onRuleChange: (ruleId: string) => void;
  onScenarioChange: (scenarioId: string) => void;
  onModeChange: (mode: string) => void;
  onAllowCommandsChange: (allowed: boolean) => void;
  onScenarioJsonChange: (json: string) => void;
  onSaveScenario: () => void;
  onGenerate: () => void;
  onRevise: () => void;
  onRun: () => void;
  onPauseToggle: () => void;
  onStep: () => void;
  onStop: () => void;
  onBack: () => void;
  onClearTrace: () => void;
  canRun: boolean;
  canGenerate: boolean;
  canRevise: boolean;
  canSaveScenario: boolean;
  canPause: boolean;
  canStep: boolean;
  canStop: boolean;
  traceLimit: number;
};

export const useRerailPlayground = ({ initialRuleId, onBack }: RerailPlaygroundOptions): RerailPlaygroundState => {
  const queryClient = useQueryClient();
  const [requestedRuleId, setRequestedRuleId] = useState(initialRuleId ?? "");
  const requestedRuleRef = useRef(requestedRuleId);
  const connectionEnabled = requestedRuleId !== "";
  const restoredRef = useRef(false);
  const [restored, setRestored] = useState(false);
  const experimentQuery = useQuery<ExperimentEvent>({ queryKey: EXPERIMENT_KEY, enabled: false });
  const experimentEvent =
    restored && experimentQuery.data?.experiment.ruleId === requestedRuleId ? experimentQuery.data : undefined;
  const experiment = experimentEvent?.experiment;
  const selection = experiment?.selection;
  const selectionRef = useRef<PlaygroundSelection | undefined>(undefined);
  const [connection, setConnection] = useState<Connection>("connecting");
  const [status, setStatus] = useState<RunStatus>({
    type: "status",
    state: "stopped",
    message: initialRuleId
      ? "Restoring the saved experiment. Opening this screen never starts a run."
      : "Choose a saved daemon rule to restore its experiment. Opening this screen never starts a run.",
  });
  const [error, setError] = useState<string | null>(null);
  const [trace, setTrace] = useState<TraceRow[]>([]);
  const [liveOutcome, setLiveOutcome] = useState<PlaygroundOutcome>();
  const [scenarioDraft, setScenarioDraft] = useState<{ base: string; text: string }>();
  const traceId = useRef(0);
  const socketRef = useRef<WebSocket | null>(null);
  const sizeRef = useRef({ cols: 100, rows: 30 });
  const generationPendingRef = useRef(false);

  const rulesQuery = useQuery({
    queryKey: rulesQueryKey,
    queryFn: ({ signal }) => fetchRules(signal),
    refetchInterval: RULES_REFRESH_INTERVAL_MS,
    retry: false,
  });
  // The socket pushes the authoritative snapshot for the connected rule; the list keeps the picker current.
  const selectedRule = experimentEvent?.rule.id === requestedRuleId
    ? experimentEvent.rule
    : rulesQuery.data?.find((rule) => rule.id === requestedRuleId);
  const ruleJson = useMemo(() => formatRuleJson(selectedRule), [selectedRule]);
  const ruleSummary = describeRule(selectedRule);
  const scenario = experiment?.scenarios.find((entry) => entry.id === selection?.scenarioId);
  const savedScenarioJson = useMemo(() => (scenario ? JSON.stringify(scenario, null, 2) : ""), [scenario]);
  const scenarioJson = scenarioDraft?.base === savedScenarioJson ? scenarioDraft.text : savedScenarioJson;
  const scenarioDirty = scenarioJson !== savedScenarioJson;

  const send = useCallback((control: PlaygroundControl) => {
    const socket = socketRef.current;
    if (socket?.readyState !== WebSocket.OPEN) return false;
    socket.send(JSON.stringify(control));
    return true;
  }, []);
  const handleInput = useCallback(
    (data: string) => {
      if (inputEncoder.encode(data).byteLength > 16_384) {
        setError("Input was not sent: a single paste exceeds 16 KiB. Paste a smaller section.");
        return;
      }
      send({ type: "input", data });
    },
    [send],
  );
  const handleResize = useCallback(
    (cols: number, rows: number) => {
      sizeRef.current = { cols: Math.max(20, Math.min(400, cols)), rows: Math.max(5, Math.min(200, rows)) };
      send({ type: "resize", ...sizeRef.current });
    },
    [send],
  );
  const { terminalHostRef, writeTerminal, resetTerminal } = usePlaygroundTerminal({
    onInput: handleInput,
    onResize: handleResize,
  });

  useEffect(() => {
    if (!connectionEnabled) return;
    let disposed = false;
    let retryTimer: ReturnType<typeof setTimeout> | undefined;
    let attempts = 0;
    const connect = () => {
      if (disposed) return;
      setConnection(attempts > 0 ? "reconnecting" : "connecting");
      const url = new URL("/__rerail_playground", globalThis.location.href);
      url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
      if (requestedRuleRef.current) url.searchParams.set("ruleId", requestedRuleRef.current);
      const socket = new WebSocket(url);
      socketRef.current = socket;
      const current = () => !disposed && socketRef.current === socket;
      socket.onopen = () => {
        if (!current()) return;
        setConnection("connected");
        setError(null);
        // The server restores its saved selection; never overwrite it with client defaults on reconnect.
        send({ type: "resize", ...sizeRef.current });
      };
      socket.onmessage = (event: MessageEvent<unknown>) => {
        if (!current()) return;
        const message = parsePlaygroundEvent(event.data);
        if (!message) {
          setError("The playground backend sent an invalid message. Reload after checking the backend version.");
          socket.close(1008, "Invalid playground event");
          return;
        }
        switch (message.type) {
          case "experiment": {
            // A rule switch may race an old run's final event; don't resurrect the old selection.
            if (requestedRuleRef.current && message.experiment.ruleId !== requestedRuleRef.current) break;
            const previous = queryClient.getQueryData<ExperimentEvent>(EXPERIMENT_KEY);
            requestedRuleRef.current = message.experiment.ruleId;
            selectionRef.current = message.experiment.selection;
            setRequestedRuleId(message.experiment.ruleId);
            const hash = `#playground?ruleId=${encodeURIComponent(message.experiment.ruleId)}`;
            if (globalThis.location.hash !== hash) globalThis.history.replaceState(null, "", hash);
            queryClient.setQueryData(EXPERIMENT_KEY, message);
            setRestored(true);
            const lastRun = traceReplacementRun(message, previous, restoredRef.current);
            if (lastRun) setTrace(lastRun.trace.map((entry) => ({ ...entry, id: ++traceId.current })).reverse());
            restoredRef.current = true;
            attempts = 0;
            break;
          }
          case "output":
            writeTerminal(message.data);
            break;
          case "reset":
            resetTerminal();
            setTrace([]);
            setLiveOutcome(undefined);
            setError(null);
            break;
          case "status":
            if (message.state === "stopped" || message.state === "error") generationPendingRef.current = false;
            setStatus(message);
            if (message.state === "error") setError(message.message);
            break;
          case "trace":
            setTrace((previous) => [{ ...message.entry, id: ++traceId.current }, ...previous].slice(0, TRACE_LIMIT));
            break;
          case "outcome":
            setLiveOutcome(message.outcome);
            break;
        }
      };
      socket.onerror = () => {
        if (current()) setError("Cannot reach the local playground backend. Check the launcher terminal.");
      };
      socket.onclose = (event) => {
        if (!current()) return;
        socketRef.current = null;
        restoredRef.current = false;
        generationPendingRef.current = false;
        setStatus({
          type: "status",
          state: "stopped",
          message: "Disconnected. The run and DB watch are stopped; saved results are retained.",
        });
        // The supervisor hands ownership to the newest tab and closes this one with 1008; retrying
        // would only fight the tab the user is looking at.
        if (event.code === 1008) {
          setConnection("disconnected");
          setError(
            (previous) => previous ?? (event.reason || "Connection rejected. Reload this tab to reclaim the experiment."),
          );
          return;
        }
        setConnection("reconnecting");
        retryTimer = setTimeout(connect, Math.min(1_000 * 2 ** Math.min(attempts++, 4), 10_000));
      };
    };
    // Let an abandoned StrictMode effect settle first; otherwise it takes ownership from this one.
    retryTimer = setTimeout(connect, 0);
    return () => {
      disposed = true;
      clearTimeout(retryTimer);
      const socket = socketRef.current;
      if (socket?.readyState === WebSocket.OPEN)
        socket.send(JSON.stringify({ type: "stop" } satisfies PlaygroundControl));
      socketRef.current = null;
      generationPendingRef.current = false;
      if (socket) {
        socket.onopen = null;
        socket.onmessage = null;
        socket.onerror = null;
        socket.onclose = null;
        socket.close();
      }
    };
  }, [connectionEnabled, queryClient, resetTerminal, send, writeTerminal]);

  const changeSelection = useCallback(
    (next: PlaygroundSelection) => {
      send({ type: "stop" });
      setError(null);
      setLiveOutcome(undefined);
      send({ type: "select", selection: next });
    },
    [send],
  );
  const onRuleChange = useCallback(
    (ruleId: string) => {
      if (ruleId === requestedRuleRef.current || !rulesQuery.data?.some((rule) => rule.id === ruleId)) return;
      const next: PlaygroundSelection = { ruleId, scenarioId: "", mode: "preview", allowCommands: false };
      requestedRuleRef.current = ruleId;
      setRequestedRuleId(ruleId);
      setRestored(false);
      restoredRef.current = false;
      selectionRef.current = undefined;
      setTrace([]);
      setScenarioDraft(undefined);
      changeSelection(next);
    },
    [changeSelection, rulesQuery.data],
  );
  const onScenarioChange = useCallback(
    (scenarioId: string) => {
      if (selectionRef.current && experiment?.scenarios.some((entry) => entry.id === scenarioId)) {
        changeSelection({ ...selectionRef.current, scenarioId });
      }
    },
    [changeSelection, experiment?.scenarios],
  );
  const onModeChange = useCallback(
    (mode: string) => {
      if (selectionRef.current && (mode === "preview" || mode === "trigger" || mode === "agent")) {
        changeSelection({ ...selectionRef.current, mode });
      }
    },
    [changeSelection],
  );
  const onAllowCommandsChange = useCallback(
    (allowCommands: boolean) => {
      if (selectionRef.current) changeSelection({ ...selectionRef.current, allowCommands });
    },
    [changeSelection],
  );
  const onScenarioJsonChange = useCallback(
    (text: string) => {
      setScenarioDraft({ base: savedScenarioJson, text });
    },
    [savedScenarioJson],
  );
  const onSaveScenario = useCallback(() => {
    if (!selectionRef.current || !scenario) return;
    let raw: unknown;
    try {
      raw = JSON.parse(scenarioJson);
    } catch {
      setError("Scenario JSON is invalid. Correct it before saving; the saved scenario is unchanged.");
      return;
    }
    const parsed = scenarioSchema.safeParse(raw);
    if (!parsed.success) {
      const issues = parsed.error.issues.map((issue) => `${issue.path.join(".")}: ${issue.message}`).join("; ");
      setError(`Scenario is invalid: ${issues}`);
      return;
    }
    if (
      parsed.data.id !== scenario.id ||
      parsed.data.ruleDigest !== scenario.ruleDigest ||
      parsed.data.ruleKind !== scenario.ruleKind
    ) {
      setError(
        "Keep the scenario id, ruleDigest, and ruleKind unchanged. Generate or Revise to derive scenarios for a changed rule.",
      );
      return;
    }
    send({ type: "stop" });
    setError(null);
    send({ type: "save-scenario", ruleId: selectionRef.current.ruleId, scenario: parsed.data });
  }, [scenario, scenarioJson, send]);
  const generate = useCallback(
    (revise: boolean) => {
      if (!selectionRef.current || generationPendingRef.current) return;
      setError(null);
      if (send({ type: "generate", ruleId: selectionRef.current.ruleId, revise })) {
        generationPendingRef.current = true;
        setStatus({
          type: "status",
          state: "generating",
          message: revise
            ? "Revising scenarios using the latest result. This uses your authenticated model account."
            : "Generating scenarios from the saved rule. This uses your authenticated model account.",
        });
      }
    },
    [send],
  );
  const onGenerate = useCallback(() => generate(false), [generate]);
  const onRevise = useCallback(() => generate(true), [generate]);
  const onRun = useCallback(() => {
    if (selectionRef.current && send({ type: "start", selection: selectionRef.current })) {
      setError(null);
      setStatus({
        type: "status",
        state: "starting",
        message: "Starting the experiment and DB watch. Stop cancels the current run and all automatic reruns.",
      });
    }
  }, [send]);
  const onPauseToggle = useCallback(() => {
    if (selectionRef.current?.mode !== "agent") send({ type: status.state === "paused" ? "resume" : "pause" });
  }, [send, status.state]);
  const onStep = useCallback(() => {
    if (selectionRef.current?.mode !== "agent") send({ type: "step" });
  }, [send]);
  const onStop = useCallback(() => {
    send({ type: "stop" });
  }, [send]);
  const handleBack = useCallback(() => {
    send({ type: "stop" });
    onBack();
  }, [onBack, send]);
  const onClearTrace = useCallback(() => setTrace([]), []);

  return {
    ...deriveCapabilities({
      connection,
      status,
      selection,
      scenario,
      scenarioDirty,
      hasLastRun: experiment?.lastRun !== undefined,
    }),
    selection,
    experiment,
    ruleSnapshot: experimentEvent?.rule,
    rules: rulesQuery.data ?? [],
    selectedRule,
    selectedRuleId: requestedRuleId,
    ruleJson,
    ruleSummary,
    scenario,
    scenarioJson,
    scenarioDirty,
    scenarioStale:
      scenario !== undefined && experimentEvent !== undefined && scenario.ruleDigest !== experimentEvent.rule.digest,
    connection,
    status,
    error: error ?? (rulesQuery.error?.message ?? null),
    trace,
    liveOutcome,
    terminalHostRef,
    onRuleChange,
    onScenarioChange,
    onModeChange,
    onAllowCommandsChange,
    onScenarioJsonChange,
    onSaveScenario,
    onGenerate,
    onRevise,
    onRun,
    onPauseToggle,
    onStep,
    onStop,
    onBack: handleBack,
    onClearTrace,
    traceLimit: TRACE_LIMIT,
  };
};
