import {
  ArrowLeftIcon,
  PauseIcon,
  PlayIcon,
  SkipForwardIcon,
  StopIcon,
  TerminalWindowIcon,
  TrashIcon,
  WarningCircleIcon,
} from "@phosphor-icons/react";

import type { PlaygroundRun } from "@protocol";
import { Badge, type BadgeVariant } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Textarea } from "@/components/ui/textarea";
import { Text, textVariants } from "@/components/ui/typography";
import { cn } from "@/lib/utils";

import type { RerailPlaygroundState } from "./useRerailPlayground";

const STATUS_BADGES: Record<RerailPlaygroundState["status"]["state"], BadgeVariant> = {
  starting: "info-soft",
  generating: "info-soft",
  running: "success-soft",
  paused: "warning-soft",
  rebuilding: "info-soft",
  stopped: "neutral-soft",
  error: "destructive-soft",
};
const traceTime = new Intl.DateTimeFormat(undefined, { hour: "2-digit", minute: "2-digit", second: "2-digit" });
const completedTime = new Intl.DateTimeFormat(undefined, { dateStyle: "medium", timeStyle: "short" });
const modeCaption = (mode: NonNullable<RerailPlaygroundState["selection"]>["mode"] | undefined): string => {
  if (mode === "agent") return "Real model reasoning and tool choices, with the selected rule evaluated naturally.";
  if (mode === "trigger") return "Replay the scenario through the actual matcher. No injected match or fallback.";
  return "Preview the selected rule’s delivery; this does not prove a detector match.";
};

const PlaygroundControls = (state: RerailPlaygroundState) => (
  <Card>
    <CardContent className="flex flex-col gap-4">
      <div className="grid gap-4 lg:grid-cols-3">
        <div className="flex min-w-0 flex-col gap-2">
          <Text variant="body-strong" id="playground-rule-label">
            Saved rule
          </Text>
          <Select
            value={state.selectedRuleId}
            onValueChange={state.onRuleChange}
            disabled={(state.selectedRuleId !== "" && state.connection !== "connected") || state.rules.length === 0}
          >
            <SelectTrigger aria-labelledby="playground-rule-label" className="w-full">
              <SelectValue placeholder="Choose a daemon rule" />
            </SelectTrigger>
            <SelectContent>
              {state.rules.map((rule) => (
                <SelectItem key={rule.id} value={rule.id}>
                  {rule.id} · {rule.kind}
                  {rule.enabled ? "" : " · saved disabled"}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <Text variant="caption">{state.ruleSummary}</Text>
        </div>
        <div className="flex min-w-0 flex-col gap-2">
          <Text variant="body-strong" id="playground-scenario-label">
            Scenario
          </Text>
          <Select
            value={state.selection?.scenarioId ?? ""}
            onValueChange={state.onScenarioChange}
            disabled={!state.experiment?.scenarios.length || state.connection !== "connected"}
          >
            <SelectTrigger aria-labelledby="playground-scenario-label" className="w-full">
              <SelectValue placeholder="Generate scenarios first" />
            </SelectTrigger>
            <SelectContent>
              {state.experiment?.scenarios.map((scenario) => (
                <SelectItem key={scenario.id} value={scenario.id}>
                  {scenario.expectation === "match" ? "Positive" : "Negative"} · {scenario.title}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <Text variant="caption" className="line-clamp-2" title={state.scenario?.explanation}>
            {state.scenario?.explanation ?? "Generate explicitly, then inspect the proposed inputs."}
          </Text>
        </div>
        <div className="flex min-w-0 flex-col gap-2">
          <Text variant="body-strong" id="playground-mode-label">
            Execution mode
          </Text>
          <Select
            value={state.selection?.mode ?? "preview"}
            onValueChange={state.onModeChange}
            disabled={!state.selection || state.connection !== "connected"}
          >
            <SelectTrigger aria-labelledby="playground-mode-label" className="w-full">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="preview">Preview delivery</SelectItem>
              <SelectItem value="trigger">Scripted trigger</SelectItem>
              <SelectItem value="agent">Real agent · paid</SelectItem>
            </SelectContent>
          </Select>
          <Text variant="caption">{modeCaption(state.selection?.mode)}</Text>
        </div>
      </div>
      <div className="flex flex-wrap items-center gap-3">
        <Button variant="outline" onClick={state.onGenerate} disabled={!state.canGenerate}>
          {state.status.state === "generating" ? "Generating scenarios…" : "Generate scenarios · paid"}
        </Button>
        <Button variant="outline" onClick={state.onRevise} disabled={!state.canRevise}>
          Revise using latest result · paid
        </Button>
        <div className="flex items-center gap-2">
          <Checkbox
            id="playground-allow-commands"
            checked={state.selection?.allowCommands ?? false}
            onCheckedChange={(checked) => state.onAllowCommandsChange(checked === true)}
            disabled={!state.selection || state.connection !== "connected"}
          />
          <label htmlFor="playground-allow-commands" className={textVariants({ variant: "body-strong" })}>
            Allow commands
          </label>
        </div>
      </div>
      {state.scenarioStale && (
        <Text variant="body" role="status" className="text-warning-foreground">
          This scenario was derived from an older rule snapshot. Runs use the latest saved rule and retain the existing
          inputs; Generate or Revise explicitly to adapt them. Incompatible inputs are reported as cannot-exercise.
        </Text>
      )}
      <div className="flex flex-wrap items-center gap-2">
        <Button onClick={state.onRun} disabled={!state.canRun}>
          <PlayIcon size={16} weight="bold" aria-hidden="true" />
          {state.selection?.mode === "agent" ? "Run real agent" : "Run"}
        </Button>
        <Button variant="outline" onClick={state.onPauseToggle} disabled={!state.canPause}>
          {state.status.state === "paused" ? (
            <PlayIcon size={16} aria-hidden="true" />
          ) : (
            <PauseIcon size={16} aria-hidden="true" />
          )}
          {state.status.state === "paused" ? "Resume" : "Pause"}
        </Button>
        <Button variant="outline" onClick={state.onStep} disabled={!state.canStep}>
          <SkipForwardIcon size={16} aria-hidden="true" />
          Step
        </Button>
        <Button variant="outline" onClick={state.onStop} disabled={!state.canStop}>
          <StopIcon size={16} aria-hidden="true" />
          Stop
        </Button>
        <Text variant="caption">
          {state.selection?.mode === "agent"
            ? "Remote generation cannot be paused. Stop cancels it."
            : "Pause and Step control scripted replay boundaries."}
        </Text>
      </div>
    </CardContent>
  </Card>
);

const PlaygroundScenario = (state: RerailPlaygroundState) => (
  <details className="rounded-lg border border-border bg-muted p-3">
    <summary className={cn(textVariants({ variant: "body-strong" }), "cursor-pointer")}>
      Scenario JSON, fixtures & scripted commands{state.scenarioDirty ? " · unsaved changes" : ""}
    </summary>
    <div className="mt-3 flex flex-col gap-3">
      {state.scenario ? (
        <>
          <Text variant="body-muted">{state.scenario.explanation}</Text>
          <Text variant="body-muted">
            Edit scenario inputs only. The rule id, kind, and definition remain read-only. Scenario id, ruleDigest, and
            ruleKind pin its origin and cannot be edited here.
          </Text>
          <label htmlFor="playground-scenario-json" className={textVariants({ variant: "body-strong" })}>
            Scenario JSON
          </label>
          <Textarea
            id="playground-scenario-json"
            value={state.scenarioJson}
            onChange={(event) => state.onScenarioJsonChange(event.target.value)}
            className={cn(textVariants({ variant: "code-block" }), "min-h-64")}
            spellCheck={false}
          />
          <div className="flex flex-wrap items-center gap-2">
            <Button variant="outline" onClick={state.onSaveScenario} disabled={!state.canSaveScenario}>
              Save scenario
            </Button>
            <Text variant="caption">
              {state.scenarioDirty
                ? "Save before running. Unsaved edits are not executed."
                : "Saved on the playground server; restored on your next visit."}
            </Text>
          </div>
          <Text variant="body-strong">Saved scripted tool calls</Text>
          {state.scenario.steps.some((step) => step.toolCalls.length > 0) ? (
            <ol className="flex flex-col gap-2">
              {state.scenario.steps.map((step, stepIndex) =>
                step.toolCalls.map((call, callIndex) => (
                  <li key={`${stepIndex}-${callIndex}`} className="flex flex-col gap-1">
                    <Text variant="caption">
                      Step {stepIndex + 1} · {call.name}
                    </Text>
                    <Text variant="code-block" className="block overflow-x-auto whitespace-pre-wrap break-words">
                      {JSON.stringify(call.arguments, null, 2)}
                    </Text>
                  </li>
                )),
              )}
            </ol>
          ) : (
            <Text variant="caption">This scenario contains no scripted tool calls.</Text>
          )}
          <Text variant="caption">
            Real-agent tool choices are not scripted; their execution is shown in the terminal and trace.
          </Text>
        </>
      ) : (
        <Text variant="body-muted">
          Generate scenarios to inspect or edit their prompt, fixtures, and replay steps.
        </Text>
      )}
    </div>
  </details>
);

const observedBadgeVariant = (run: PlaygroundRun): BadgeVariant => {
  const { observed } = run.outcome;
  if (observed === "error") return "destructive-soft";
  if (observed === "cannot-exercise" || observed === "preview") return "warning-soft";
  return observed === run.scenario.expectation ? "success-soft" : "destructive-soft";
};
const observedSummary = (run: PlaygroundRun): string => {
  const { observed } = run.outcome;
  if (observed === "cannot-exercise") return "Capability gap—not a negative match. ";
  if (observed === "error") return "Execution failed—not a negative match. ";
  if (observed === "preview") return "Delivery preview—not a matching assertion. ";
  return observed === run.scenario.expectation ? "Expectation met. " : "Expectation not met. ";
};

const CompletedRunSummary = ({ run }: { run: PlaygroundRun }) => (
  <>
    <div className="flex flex-wrap items-center gap-2">
      <Badge variant="outline">Expected: {run.scenario.expectation}</Badge>
      <Badge variant={observedBadgeVariant(run)}>Observed: {run.outcome.observed}</Badge>
      <Text variant="caption">{completedTime.format(run.completedAt)}</Text>
    </div>
    <Text variant="body">
      {observedSummary(run)}
      {run.outcome.detail}
    </Text>
    <Text variant="caption">
      {run.scenario.title} · {run.selection.mode} · rule snapshot {run.rule.digest.slice(0, 12)}
    </Text>
    <details>
      <summary className={cn(textVariants({ variant: "caption" }), "cursor-pointer")}>
        Pinned run inputs and rule snapshot
      </summary>
      <Text variant="code-block" className="mt-2 block max-h-80 overflow-auto whitespace-pre-wrap break-words">
        {JSON.stringify({ selection: run.selection, rule: run.rule, scenario: run.scenario }, null, 2)}
      </Text>
    </details>
  </>
);

const PlaygroundResult = ({ experiment }: Pick<RerailPlaygroundState, "experiment">) => (
  <div className="flex flex-col gap-2 rounded-lg border border-border p-3" aria-label="Latest completed experiment">
    <Text variant="body-strong">Latest completed experiment</Text>
    {experiment?.lastRun ? (
      <CompletedRunSummary run={experiment.lastRun} />
    ) : (
      <Text variant="caption">No completed run yet. Generate or select a scenario, then choose Run explicitly.</Text>
    )}
  </div>
);

const PlaygroundInspector = (state: RerailPlaygroundState) => (
  <Card className="min-h-0 min-w-0">
    <CardContent>
      <Tabs defaultValue="rule">
        <TabsList aria-label="Playground inspector">
          <TabsTrigger value="rule">Rule</TabsTrigger>
          <TabsTrigger value="trace">Event trace</TabsTrigger>
        </TabsList>
        <TabsContent value="rule" className="flex flex-col gap-3">
          <Text variant="subtitle">Saved rule · read-only</Text>
          <Text variant="body-muted">
            Edit the rule through your LLM’s DB tools. This detail refreshes regularly from the daemon.
          </Text>
          <Text variant="caption">{state.ruleSummary}</Text>
          <Badge variant={state.selectedRule?.enabled ? "success-soft" : "neutral-soft"}>
            {state.selectedRule?.enabled ? "Enabled in DB" : "Disabled in DB"}
          </Badge>
          <Text variant="caption">
            Only this rule is activated in the fresh sandbox—even when saved disabled. The DB is not modified.
          </Text>
          <Text
            variant="code-block"
            className="block max-h-[60vh] overflow-auto whitespace-pre-wrap break-words rounded-lg bg-muted p-3"
          >
            {state.ruleJson || "Waiting for rule details."}
          </Text>
        </TabsContent>
        <TabsContent value="trace" className="flex flex-col gap-3">
          <div className="flex items-start justify-between gap-2">
            <Text variant="caption">
              Newest first · last {state.traceLimit} events. Saved run trace restores on reconnect.
            </Text>
            <Button
              variant="ghost"
              size="icon"
              onClick={state.onClearTrace}
              disabled={state.trace.length === 0}
              aria-label="Clear displayed event trace"
            >
              <TrashIcon size={16} aria-hidden="true" />
            </Button>
          </div>
          <div className="max-h-[65vh] overflow-y-auto">
            {state.trace.length === 0 ? (
              <Text variant="body-muted">Waiting for OMP and Rerail events. Terminal bytes are shown separately.</Text>
            ) : (
              <ol aria-label="OMP and Rerail event trace" className="flex flex-col gap-3">
                {state.trace.map((entry) => (
                  <li key={entry.id} className="flex flex-col gap-1 border-b border-subtle pb-3">
                    <div className="flex flex-wrap items-center gap-2">
                      <Text variant="meta">
                        {Number.isFinite(entry.at) && Math.abs(entry.at) <= 8_640_000_000_000_000
                          ? traceTime.format(entry.at)
                          : "Unknown time"}
                      </Text>
                      <Badge variant="outline">{entry.kind}</Badge>
                    </div>
                    <Text variant="code-block" className="whitespace-pre-wrap break-words">
                      {entry.text}
                    </Text>
                  </li>
                ))}
              </ol>
            )}
          </div>
        </TabsContent>
      </Tabs>
    </CardContent>
  </Card>
);

export const RerailPlayground = (state: RerailPlaygroundState) => (
  <main className="flex min-h-dvh flex-col gap-4 bg-background p-4 text-foreground sm:p-6">
    <header className="flex flex-wrap items-start justify-between gap-4">
      <div className="flex flex-col gap-2">
        <Button variant="ghost" onClick={state.onBack} className="self-start">
          <ArrowLeftIcon size={16} aria-hidden="true" />
          Back to Rules
        </Button>
        <Text variant="overline">Rerail · development only</Text>
        <Text variant="headline" className="flex items-center gap-2">
          <TerminalWindowIcon size={24} aria-hidden="true" />
          Rule playground
        </Text>
        <Text variant="body-muted">
          Exercise one actual saved rule in a fresh OMP sandbox. Preserve the evidence, not a synthetic match.
        </Text>
      </div>
      <div className="flex flex-wrap items-center gap-2" role="status" aria-live="polite">
        <Badge variant={state.connection === "connected" ? "success-soft" : "warning-soft"}>{state.connection}</Badge>
        <Badge variant={STATUS_BADGES[state.status.state]}>{state.status.state}</Badge>
      </div>
    </header>
    <PlaygroundControls {...state} />
    <Text variant="caption" as="p" role="status">
      {state.status.message}
    </Text>
    {state.error && (
      <div role="alert" className="flex items-start gap-2 rounded-lg border border-danger bg-danger-subtle p-3">
        <WarningCircleIcon size={24} className="shrink-0 text-danger-foreground" aria-hidden="true" />
        <Text variant="body" className="text-danger-foreground">
          {state.error}
        </Text>
      </div>
    )}
    {state.liveOutcome && (
      <Text variant="body" role="status">
        Live observation: {state.liveOutcome.observed} · {state.liveOutcome.detail}
      </Text>
    )}
    <div className="grid min-h-0 grid-cols-1 items-start gap-4 lg:grid-cols-[minmax(0,2fr)_minmax(18rem,1fr)]">
      <div className="flex min-w-0 flex-col gap-4">
        <Card className="min-w-0">
          <CardHeader>
            <Text variant="subtitle">Live OMP terminal</Text>
            <Text variant="caption">
              Click to type into the isolated process. In real-agent mode additional prompts incur account usage.
            </Text>
          </CardHeader>
          <CardContent>
            <section
              ref={state.terminalHostRef}
              aria-label="Live OMP terminal"
              className={cn(
                textVariants({ variant: "code-block" }),
                "h-[55vh] min-h-80 w-full overflow-hidden bg-background lg:h-[65vh]",
              )}
            />
          </CardContent>
        </Card>
        <PlaygroundResult experiment={state.experiment} />
        <PlaygroundScenario {...state} />
      </div>
      <PlaygroundInspector {...state} />
    </div>
  </main>
);
