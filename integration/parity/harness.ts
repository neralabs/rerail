export const PARITY_HOSTS = ["omp", "pi", "claude-code", "codex", "cursor"] as const;
export type ParityHost = (typeof PARITY_HOSTS)[number];

export const REQUIRED_CAPABILITIES = [
  "event-capture",
  "transcript-persistence",
  "configured-rule-evaluation",
  "repeat-policy",
  "automatic-delivery",
  "rule-management",
  "semantic-workspace",
  "persistence-replay",
  "session-isolation",
  "malformed-event-containment",
  "performance-budget",
] as const;
export type RequiredCapability = (typeof REQUIRED_CAPABILITIES)[number];

export type ParityStep = Readonly<{ kind: string } & Record<string, unknown>>;
export type ParityObservation = Readonly<{ kind: string } & Record<string, unknown>>;

export interface ParityRun {
  step(step: ParityStep): Promise<readonly ParityObservation[]>;
  close(): Promise<void>;
}

export interface ParityAdapter {
  readonly host: ParityHost;
  open(): Promise<ParityRun>;
}

export interface CapabilityScenario {
  readonly capability: RequiredCapability;
  execute(run: ParityRun, host: ParityHost): Promise<void>;
}

export interface ParityCase {
  readonly name: `[${ParityHost}] ${RequiredCapability}`;
  run(): Promise<void>;
}

function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function assertCompleteAdapters(adapters: readonly ParityAdapter[]): void {
  const exact = adapters.length === PARITY_HOSTS.length
    && PARITY_HOSTS.every(host => adapters.filter(adapter => adapter.host === host).length === 1);
  if (!exact) {
    throw new Error(`Parity adapters must contain exactly one of: ${PARITY_HOSTS.join(", ")}`);
  }
}

function assertCompleteScenarios(scenarios: readonly CapabilityScenario[]): void {
  for (const capability of REQUIRED_CAPABILITIES) {
    const count = scenarios.filter(scenario => scenario.capability === capability).length;
    if (count === 0) throw new Error(`Missing required parity scenario: ${capability}`);
    if (count > 1) throw new Error(`Duplicate parity scenario: ${capability}`);
  }
  if (scenarios.length !== REQUIRED_CAPABILITIES.length) {
    throw new Error("Parity scenarios contain an unknown capability");
  }
}

async function executeCase(adapter: ParityAdapter, scenario: CapabilityScenario): Promise<void> {
  const label = `[${adapter.host}] ${scenario.capability}`;
  let run: ParityRun;
  try {
    run = await adapter.open();
  } catch (error) {
    throw new Error(`${label}: ${message(error)}`, { cause: error });
  }

  let failure: unknown;
  try {
    await scenario.execute(run, adapter.host);
  } catch (error) {
    failure = error;
  }
  try {
    await run.close();
  } catch (error) {
    if (failure === undefined) failure = error;
  }
  if (failure !== undefined) {
    throw new Error(`${label}: ${message(failure)}`, { cause: failure });
  }
}

export function parityMatrix(input: {
  readonly adapters: readonly ParityAdapter[];
  readonly scenarios: readonly CapabilityScenario[];
}): readonly ParityCase[] {
  assertCompleteAdapters(input.adapters);
  assertCompleteScenarios(input.scenarios);
  const adapters = new Map(input.adapters.map(adapter => [adapter.host, adapter]));
  const scenarios = new Map(input.scenarios.map(scenario => [scenario.capability, scenario]));

  return REQUIRED_CAPABILITIES.flatMap(capability => PARITY_HOSTS.map(host => ({
    name: `[${host}] ${capability}` as const,
    run: () => executeCase(adapters.get(host)!, scenarios.get(capability)!),
  })));
}
