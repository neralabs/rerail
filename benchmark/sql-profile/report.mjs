import { createHash } from "node:crypto";
import { createReadStream, readFileSync, readdirSync, openSync, closeSync, writeSync, writeFileSync } from "node:fs";
import { createInterface } from "node:readline";
import { join } from "node:path";
import { actionableSummary, summaryMarkdown } from "./summary.mjs";
import { executionBudget, JOB_SQL_EXECUTION_LIMIT } from "./execution-budget.mjs";
import { controlStages } from "./cte-steps.mjs";

export const thresholds = Object.freeze({ highVmPerRow: 1000, minimumVm: 100_000, emptyVm: 10_000,
  repeatedRuns: 100, smallCpuNs: 1_000_000, expansion: 10, expandedRows: 1000, indexBuilds: 10,
  frameAdvances: 100_000, repeatedScans: 100 });
const fields = ["runs", "cpuNs", "wallNs", "selfCpuNs", "selfWallNs", "rows", "changes", "vmSteps", "fullscanSteps", "sorts", "autoIndexRows",
  "frameScans", "frameLookups", "frameNext", "frameColumns", "frameInserts", "frameIndexBuilds", "frameIndexedRows", "frameIndexCpuNs", "frameIndexWallNs", "frameUpdatedCells", "frameIndexTimedBuilds"];
const frameFields = fields.slice(11, -1);
const counterFields = ["vmSteps", "fullscanSteps", "sorts", "autoIndexRows"];
const ms = ns => (ns / 1e6).toFixed(3);

export function flags(query) {
  const out = [];
  const add = (code, certainty, evidence, action) => out.push({ code, certainty, evidence, action });
  const plan = query.plan ?? query.plans?.[0]?.plan, bytecode = query.bytecode ?? query.plans?.[0]?.bytecode;
  const details = Array.isArray(plan) ? plan.map(row => row[3]).join("\n") : "";
  if (query.kind === "sql" && (!Array.isArray(plan) || !Array.isArray(bytecode))) {
    add("explain-unavailable", "observed", query.plan?.error ?? query.bytecode?.error ?? "Missing EXPLAIN", "Inspect the raw SQL and schema lifetime.");
  }
  if (/USE TEMP B-TREE/i.test(details)) add("temporary-btree", "potential", details.split("\n").filter(s => /TEMP B-TREE/i.test(s)), "Inspect ordering, grouping, and deduplication before materialization.");
  if (/CORRELATED.*SUBQUERY/i.test(details)) add("correlated-subquery", "potential", details.split("\n").filter(s => /CORRELATED/i.test(s)), "Inspect whether this subquery repeats for each outer row.");
  if (/MATERIALIZE /i.test(details)) add("materialization", "potential", details.split("\n").filter(s => /MATERIALIZE /i.test(s)), "Inspect reuse and push restrictions into the materialized input.");
  if (query.fullscanSteps) add("full-scan", "observed", { fullscanSteps: query.fullscanSteps }, "Inspect missing restrictions or lookup indexes.");
  if (/SCAN .*VIRTUAL TABLE INDEX 0:/i.test(details)) add("virtual-full-scan", "potential", { frameScans: query.frameScans, frameNext: query.frameNext }, "Inspect the virtual-table plan and measured frame visits.");
  if (query.sorts) add("runtime-sorts", "observed", { sorts: query.sorts }, "Inspect ordering and grouping in this query.");
  if (query.autoIndexRows) add("automatic-index", "observed", { insertedIndexRows: query.autoIndexRows }, "Inspect whether repeated executions rebuild the same automatic index.");
  if (query.frameIndexBuilds >= thresholds.indexBuilds) add("frame-index-rebuilds", "observed", { builds: query.frameIndexBuilds, indexedRows: query.frameIndexedRows }, "Inspect frame writes that invalidate lookup indexes.");
  const output = query.rows + query.changes;
  if (query.vmSteps >= thresholds.minimumVm && query.vmSteps / Math.max(1, output) >= thresholds.highVmPerRow) add("high-vm-per-row", "observed", { vmSteps: query.vmSteps, resultOrChangedRows: output }, "Inspect repeated probes and intermediate work per result.");
  if (!output && query.vmSteps >= thresholds.emptyVm) add("empty-result-work", "observed", { vmSteps: query.vmSteps, cpuMs: Number(ms(query.cpuNs)) }, "Inspect whether an earlier emptiness check can avoid this work.");
  if (query.runs >= thresholds.repeatedRuns && query.cpuNs / query.runs < thresholds.smallCpuNs) add("many-small-executions", "observed", { runs: query.runs, averageCpuMs: Number(ms(query.cpuNs / query.runs)) }, "Inspect batching or fusion across these executions.");
  if (query.maxExpansion >= thresholds.expansion) add("row-expansion", "observed", { maximumOutputToLargestInput: query.maxExpansion }, "Inspect join multiplicity and push candidate restrictions earlier.");
  if (query.frameScans >= thresholds.repeatedScans && query.frameNext >= thresholds.frameAdvances) add("repeated-frame-scans", "observed", { scans: query.frameScans, cursorAdvances: query.frameNext }, "Inspect keyed lookup opportunities and repeated input scans.");
  if (query.incomplete) add("incomplete-operation", "observed", { executions: query.incomplete }, "Inspect workload failures before interpreting performance.");
  return out;
}

export async function buildReport(rawDirectory, outputDirectory, workload = {}) {
  const files = readdirSync(rawDirectory).filter(name => /^profile-.*\.json$/.test(name)).sort();
  if (!files.length) throw new Error("No SQL sessions were captured. Select supported SQL clauses and build the addon with --features unbounded-work.");
  const byText = new Map();
  const queries = [];
  const contexts = new Map();
  const relationalStages = new Map();
  const executionCoverage = [], firstExecutions = [];
  const clauseNames = new Map((workload.results?.sql?.report?.compiledClauseNames ?? workload.results?.sql?.report?.profile?.clauses ?? []).map(clause => [clause.statement, clause]));
  const trace = openSync(join(outputDirectory, "trace.json"), "w");
  writeSync(trace, '{"displayTimeUnit":"ms","traceEvents":[');
  let first = true, eventCount = 0;
  try {
    for (const name of files) {
      const catalogue = JSON.parse(readFileSync(join(rawDirectory, name), "utf8"));
      if (catalogue.version !== 1) throw new Error(`Unsupported SQL profile version: ${catalogue.version}`);
      for (const batch of catalogue.relationalBatches ?? []) {
        for (const stage of batch.stagesByCpu ?? []) {
          const key = JSON.stringify([stage.sql, stage.columns]);
          let combined = relationalStages.get(key);
          if (!combined) {
            combined = { sql: stage.sql, columns: stage.columns, cpuNs: 0, wallNs: 0, runs: 0, incomplete: 0 };
            relationalStages.set(key, combined);
          }
          combined.cpuNs += stage.cpuNs;
          combined.wallNs += stage.wallNs;
          combined.runs++;
          if (stage.incomplete) combined.incomplete++;
        }
      }
      const local = new Map(catalogue.queries.map(query => {
        const key = `${query.kind}:${query.sql}`;
        let combined = byText.get(key);
        if (!combined) {
          combined = { ...query, bytecode: undefined, id: queries.length, ...Object.fromEntries(fields.map(f => [f, 0])), maxExpansion: 0, incomplete: 0, contexts: new Set(), plans: [] };
          queries.push(combined); byText.set(key, combined);
        }
        if (query.kind === "sql") {
          // VOpen embeds connection pointers. Those addresses do not define a different
          // physical plan. Keep genuine variants and reference every original raw capture.
          const identity = createHash("sha256").update(JSON.stringify(query.plan) ?? "null").update(JSON.stringify(query.bytecode,
            (_, value) => typeof value === "string" && /^vtab:[0-9a-f]+$/i.test(value) ? "vtab:connection-address" : value) ?? "null").digest("hex");
          let variant = combined.plans.find(p => p.identity === identity);
          if (!variant) { variant = { identity, plan: query.plan, bytecode: query.bytecode, captures: [] }; combined.plans.push(variant); }
          variant.captures.push({ catalogue: `raw/${name}`, query: query.id });
        }
        return [query.id, combined];
      }));
      let localEvents = 0;
      let localSql = 0;
      for await (const line of createInterface({ input: createReadStream(join(rawDirectory, name.replace(/\.json$/, ".jsonl"))), crlfDelay: Infinity })) {
        if (!line) continue;
        const event = JSON.parse(line);
        const query = local.get(event.query);
        if (!query) throw new Error(`Missing query ${event.query} in ${name}`);
        eventCount++; localEvents++;
        if (event.kind !== query.kind) throw new Error(`Trace kind mismatch in ${name}`);
        if (event.kind === "sql") {
          localSql++;
          // Only the first 101 per session can contain the job's execution 101.
          if (localSql <= JOB_SQL_EXECUTION_LIMIT + 1) firstExecutions.push({
            startNs: event.startNs, session: name, query: query.id, sql: query.sql,
          });
        }
        query.runs++;
        if (event.preparation) {
          query.preparation ??= {cpuNs:0,wallNs:0,runs:0,maxStatementMemoryBytes:0};
          query.preparation.cpuNs += event.preparation.cpuNs;
          query.preparation.wallNs += event.preparation.wallNs;
          query.preparation.runs++;
          query.preparation.maxStatementMemoryBytes=Math.max(query.preparation.maxStatementMemoryBytes,event.preparation.statementMemoryBytes);
        }
        for (const key of fields.slice(1, 8)) query[key] += event[key] ?? 0;
        counterFields.forEach((key, i) => { query[key] += event.counters?.[i] ?? 0; });
        frameFields.forEach((key, i) => { query[key] += event.frame?.[i] ?? 0; });
        if (event.frame?.length >= 9) query.frameIndexTimedBuilds += event.frame[5];
        if (event.kind === "operator" && !event.completed) query.incomplete++;
        const context = { ...(event.context ?? {}) };
        if (context.operator !== undefined) context.operator = local.get(context.operator)?.id ?? context.operator;
        const clause = context.scope === "clause" ? clauseNames.get(context.statement) : undefined;
        if (clause) Object.assign(context, { ruleId: clause.ruleId, clauseId: clause.clauseId });
        const largest = Math.max(0, ...(context.inputs ?? []).map(([, count]) => count));
        if (event.kind === "operator" && largest > 0 && event.rows >= thresholds.expandedRows) query.maxExpansion = Math.max(query.maxExpansion, event.rows / largest);
        const scopeKey = context.scope ? `${context.scope}:${context.statement}${clause ? ` ${clause.ruleId}/${clause.clauseId}` : ""}` : "setup/input/cleanup";
        query.contexts.add(scopeKey);
        let scope = contexts.get(scopeKey);
        if (!scope) { scope = { scope: context.scope ?? "session", statement: context.statement, ruleId: clause?.ruleId, clauseId: clause?.clauseId, selfCpuNs: 0, selfWallNs: 0, executions: 0 }; contexts.set(scopeKey, scope); }
        scope.selfCpuNs += event.selfCpuNs; scope.selfWallNs += event.selfWallNs; scope.executions++;
        const traceEvent = { name: `Q${query.id} ${event.kind}`, cat: event.kind, ph: "X", pid: catalogue.pid,
          tid: event.thread, ts: event.startNs / 1000, dur: event.wallNs / 1000,
          args: { query: query.id, sql: query.sql, cpuMs: event.cpuNs / 1e6, selfCpuMs: event.selfCpuNs / 1e6,
            rows: event.rows, changes: event.changes, context, counters: event.counters, frame: event.frame,
            flags: flags(query).map(flag => flag.code), flagBasis: "query totals through this event" } };
        writeSync(trace, (first ? "" : ",") + JSON.stringify(traceEvent)); first = false;
      }
      if (localEvents !== catalogue.events) throw new Error(`Incomplete trace ${name}: ${localEvents}/${catalogue.events} events`);
      executionCoverage.push({ session: name, pid: catalogue.pid, ...catalogue.executionCoverage, observed: localSql });
    }
    writeSync(trace, "]}");
  } finally { closeSync(trace); }
  for (const query of queries) {
    query.contexts = [...query.contexts].sort();
    // Union plan hazards across all observed plans.
    const allFlags = [flags(query), ...query.plans.map(plan => flags({ ...query, ...plan }))].flat();
    query.flags = [...new Map(allFlags.map(flag => [flag.code, flag])).values()];
  }
  queries.sort((a, b) => b.selfCpuNs - a.selfCpuNs || a.id - b.id);
  const summary = Object.fromEntries(fields.map(key => [key, queries.reduce((n, q) => n + (key === "cpuNs" || key === "wallNs" ? (q.kind === "sql" ? q[key] : 0) : q[key]), 0)]));
  firstExecutions.sort((a, b) => a.startNs - b.startNs || a.session.localeCompare(b.session));
  const report = { version: 1, workload, thresholds, eventCount, sessions: files.length, summary,
    executionCoverage, firstOverBudget: firstExecutions[JOB_SQL_EXECUTION_LIMIT] ?? null,
    accounting: "Ranked CPU uses SQL CPU plus operator CPU excluding child SQL. SQL spans exclude preparation; operator spans include preparation. Parsing and other pipeline phases outside these spans are not attributed. Inclusive operator times overlap SQL. Aggregate wall time sums worker spans, not workload latency. Profiling callback and serialization overhead affect timings. EXPLAIN uses unbound NULL parameters after execution. Query plans describe SQLite choices, not actual loop counts. Frame counters cover frame tables; terminal internals are included in SQL time but have no callback breakdown. Rows are returned, changed, or published rows; shared rows are not copies. Summed row counts include operator and SQL observations of the same rows. Threshold flags are review candidates, not correctness failures.",
    relationalStages: [...relationalStages.values(), ...await controlStages(rawDirectory,clauseNames)].sort((a, b) => b.cpuNs - a.cpuNs),
    contexts: [...contexts.values()].sort((a, b) => b.selfCpuNs - a.selfCpuNs), queries };
  return writeReport(report, outputDirectory);
}

/** Regenerates presentation and priorities without rerunning or rewriting the execution trace. */
export function writeReport(report, outputDirectory) {
  report.executionBudget = executionBudget(report);
  report.actionableSummary = actionableSummary(report);
  if (report.executionBudget.status === "budget-exceeded") {
    report.actionableSummary.nextStep = "Meet the complete-job SQL execution budget before CPU tuning. Preserve complete native output parity.";
  }
  const hash = path => createHash("sha256").update(readFileSync(path)).digest("hex");
  report.analysis = { version: 1, reporterSha256: hash(new URL(import.meta.url)),
    executionBudgetSha256: hash(new URL("./execution-budget.mjs", import.meta.url)),
    controlStageReporterSha256: hash(new URL("./cte-steps.mjs", import.meta.url)),
    summarySha256: hash(new URL("./summary.mjs", import.meta.url)) };
  writeFileSync(join(outputDirectory, "summary.md"), summaryMarkdown(report));
  writeFileSync(join(outputDirectory, "execution-budget.json"), JSON.stringify(report.executionBudget, null, 2) + "\n");
  writeJsonReport(join(outputDirectory, "profile.json"), report);
  const lines = [summaryMarkdown(report), "", "# Complete query catalogue", "", `Output parity: ${report.workload?.results?.parity ?? "unavailable"}.`, "", report.accounting, "",
    `Events: ${report.eventCount}. Sessions: ${report.sessions}. Accounted CPU: ${ms(report.summary.selfCpuNs)} ms.`, "",
    "| Query | Kind | Self CPU ms | Runs | Rows | VM steps | Flags |", "|---|---|---:|---:|---:|---:|---|"];
  for (const q of report.queries) lines.push(`| Q${q.id} | ${q.kind} | ${ms(q.selfCpuNs)} | ${q.runs} | ${q.rows + q.changes} | ${q.vmSteps} | ${q.flags.map(f => f.code).join(", ")} |`);
  for (const q of report.queries) {
    lines.push("", `## Q${q.id}`, "", `Contexts: ${q.contexts.join(", ")}.`, "", "```sql", q.sql, "```", "");
    for (const flag of q.flags) lines.push(`- **${flag.code} (${flag.certainty})**: ${JSON.stringify(flag.evidence)}. ${flag.action}`);
    if (q.kind === "sql") lines.push("", "```json", JSON.stringify(q.plans.map(p => p.plan), null, 2), "```");
  }
  writeFileSync(join(outputDirectory, "profile.md"), lines.join("\n") + "\n");
  const page = openSync(join(outputDirectory, "profile.html"), "w");
  try { for (const chunk of html(report)) writeSync(page, chunk); } finally { closeSync(page); }
  return report;
}

// A complete EXPLAIN catalogue can exceed V8's maximum string length. Write independent
// top-level records without constructing one string for the entire report.
function writeJsonReport(path, report) {
  const file = openSync(path, "w");
  try {
    writeSync(file, "{");
    let first = true;
    for (const [key, value] of Object.entries(report)) {
      writeSync(file, `${first ? "" : ","}\n${JSON.stringify(key)}:`); first = false;
      if (Array.isArray(value)) {
        writeSync(file, "[");
        for (let i = 0; i < value.length; i++) writeSync(file, `${i ? "," : ""}\n${JSON.stringify(value[i])}`);
        writeSync(file, "]");
      } else writeSync(file, JSON.stringify(value));
    }
    writeSync(file, "\n}\n");
  } finally { closeSync(file); }
}

function* html(report) {
  const overview = { ...report, workload: { results: { parity: report.workload?.results?.parity } },
    queries: report.queries.map(({plan,bytecode,plans,...query}) => query) };
  const payload = JSON.stringify(overview).replaceAll("<", "\\u003c").replaceAll("\u2028", "\\u2028").replaceAll("\u2029", "\\u2029");
  yield `<!doctype html><html lang="en"><meta charset="utf-8"><title>SQL workload profile</title>
<style>body{font:14px system-ui;margin:24px;color:#17202a}table{border-collapse:collapse;width:100%}th,td{text-align:left;padding:8px;border-bottom:1px solid #ddd}th{cursor:pointer}tr:hover{background:#eef5ff}pre{white-space:pre-wrap;overflow-wrap:anywhere;background:#f4f6f8;padding:12px}.flag{color:#934500}input{padding:8px;width:70%}#detail{border-top:2px solid #aaa;margin-top:24px}button{margin:8px;padding:6px}.action{border:1px solid #ccd4de;border-radius:8px;padding:12px;margin:12px 0}.cost{font-weight:600}.muted{color:#52606d}#priorities table{margin:12px 0}details{margin:12px 0}</style>
<h1>SQL workload profile</h1><p id="summary"></p><section id="priorities"></section><details><summary>Measurement limits</summary><p id="accounting"></p></details><details><summary>Work inside fused SQL</summary><p>These stage times are included in SQL CPU. Select a stage to inspect its SQL.</p><div id="relational"></div></details><h2>All queries</h2><input id="filter" aria-label="Filter queries" placeholder="Filter SQL, scope, or flag"><button id="flagged">Flagged only</button><table><thead><tr><th data-sort="id">Query</th><th data-sort="selfCpuNs">Self CPU ms ↓</th><th data-sort="runs">Runs</th><th data-sort="rows">Rows</th><th data-sort="vmSteps">VM steps</th><th>Flags</th></tr></thead><tbody id="rows"></tbody></table><section id="detail"></section>
<script type="application/json" id="data">${payload}</script>`;
  for (const query of report.queries) {
    const plans = JSON.stringify(query.plans).replaceAll("<", "\\u003c").replaceAll("\u2028", "\\u2028").replaceAll("\u2029", "\\u2029");
    yield `<script type="application/json" id="plans-${query.id}">${plans}</script>`;
  }
  yield `<script>
const report=JSON.parse(document.getElementById('data').textContent);let sort='selfCpuNs',direction=-1,flagged=false;
const el=id=>document.getElementById(id),ms=n=>(n/1e6).toFixed(3),text=(tag,value)=>{const e=document.createElement(tag);e.textContent=value;return e};
el('summary').textContent=report.eventCount+' events · '+report.sessions+' sessions · '+ms(report.summary.selfCpuNs)+' ms accounted CPU · output parity: '+(report.workload.results?.parity??'unavailable');el('accounting').textContent=report.accounting;
if(report.executionBudget){const b=report.executionBudget;el('summary').prepend(text('strong','SQL job budget: '+b.status+' ('+b.observed+'/'+b.limit+'). '));}
function detail(q){const d=el('detail');d.replaceChildren(text('h2','Q'+q.id+' · '+q.kind),text('p',q.contexts.join(', ')),text('pre',q.sql));for(const f of q.flags)d.append(text('p',f.code+' ('+f.certainty+'): '+JSON.stringify(f.evidence)+' '+f.action));d.append(text('h3','Metrics'),text('pre',JSON.stringify({...q,sql:undefined,plans:undefined,bytecode:undefined,plan:undefined},null,2)));if(q.kind==='sql')d.append(text('h3','EXPLAIN QUERY PLAN and bytecode'),text('pre',JSON.stringify(JSON.parse(el('plans-'+q.id).textContent),null,2)));d.scrollIntoView({block:'start'});}
function render(){const filter=el('filter').value.toLowerCase();el('rows').replaceChildren();for(const q of [...report.queries].filter(q=>(!flagged||q.flags.length)&&JSON.stringify([q.sql,q.contexts,q.flags]).toLowerCase().includes(filter)).sort((a,b)=>direction*(a[sort]-b[sort]))){const r=document.createElement('tr');for(const value of ['Q'+q.id+' '+q.kind,ms(q.selfCpuNs),q.runs,q.rows+q.changes,q.vmSteps,q.flags.map(f=>f.code).join(', ')])r.append(text('td',value));r.onclick=()=>detail(q);el('rows').append(r);}}
const a=report.actionableSummary,p=el('priorities'),fmt=n=>n.toLocaleString('en-US',{maximumFractionDigits:n>0&&n<1?3:1});
for(const [i,s] of (report.relationalStages??[]).slice(0,20).entries()){const d=document.createElement('details'),h=document.createElement('summary'),b=document.createElement('pre');h.textContent='S'+i+(s.source?' · '+(s.source.ruleId?s.source.ruleId+'/'+s.source.clauseId:s.source.scope+':'+s.source.statement):'')+': '+fmt(s.cpuNs/1e6)+' ms, '+s.runs+' runs, '+s.columns.length+' columns'+(s.publishedRows===undefined&&s.updatedRows===undefined?'':', '+fmt(s.publishedRows??0)+' published rows, '+fmt(s.frameNext)+' frame advances, '+fmt(s.frameUpdatedCells??0)+' updated cells')+(s.incomplete?' (incomplete)':'');b.textContent=s.sql+(s.nextSql?'\\n\\n-- Next control state:\\n'+s.nextSql:'');d.append(h,b);el('relational').append(d);}
p.append(text('p',a.nextStep),text('p',a.budget.interpretation),text('h2','Where recorded CPU goes'));
const costs=document.createElement('table');for(const c of a.costBreakdown){const row=document.createElement('tr');for(const v of [c.label,fmt(c.cpuMs)+' ms',fmt(c.measuredCpuPercent)+'%',fmt(c.executions)+' executions'])row.append(text('td',v));costs.append(row);}if(a.budget.preparationCpuMs!==null&&a.budget.preparationCpuMs!==undefined)p.append(text('p','Measured preparation/cache lookup: '+fmt(a.budget.preparationCpuMs)+' ms (included in operator CPU); largest statement '+fmt(a.budget.largestStatementMemoryBytes/1048576)+' MiB before execution.'));
p.append(costs,text('p',a.budget.frameIndexCpuMs===null?'Frame index construction was not timed in this capture. Re-run with the current profiler.':'Frame index construction: '+fmt(a.budget.frameIndexCpuMs)+' ms across '+fmt(a.budget.frameIndexTimedBuilds)+' timed builds ('+fmt(a.budget.frameIndexBuilds)+' total), including scan-order preparation on cache misses. Already included in SQL CPU.'),text('h2','Priority actions'),text('p',a.interpretation));
if(!a.actions.length)p.append(text('p','No action group reaches both 10 ms and 1% of recorded CPU. Use a larger representative capture.'));
for(const action of a.actions){const card=text('article','');card.className='action';card.append(text('h3',action.rank+'. '+action.title),text('p',fmt(action.cpuMs)+' ms · '+fmt(action.measuredCpuPercent)+'% of recorded CPU · '+fmt(action.executions)+' executions'),text('p','Evidence: '+action.certainty+'. '+action.reason),text('p','Next change: '+action.change),text('p','Code: '+action.source),text('p','Check: '+action.verify));for(const id of action.representativeQueryIds){const button=text('button','Q'+id);button.onclick=()=>detail(report.queries.find(q=>q.id===id));card.append(button);}p.append(card);}
p.append(text('h2','Root-input calls'),text('p','Published '+fmt(a.rootInput.publication.returnedRows)+' candidate memberships in '+fmt(a.rootInput.publication.executions)+' batch operations ('+fmt(a.rootInput.publication.cpuMs)+' ms).'),text('p',fmt(a.rootInput.executions)+' calls · '+fmt(a.rootInput.sqlExecutionPercent)+'% of SQL executions · '+fmt(a.rootInput.measuredCpuPercent)+'% of recorded CPU'),text('p',a.rootInput.explanation),text('p','Next change: '+a.rootInput.nextChange));
p.append(text('p',a.quality.expectedSetupExplainFailures+' expected setup explanation failures · '+a.quality.unexpectedExplainFailures+' unexpected explanation failures · '+a.quality.incompleteOperations+' incomplete operations'));
el('filter').oninput=render;el('flagged').onclick=()=>{flagged=!flagged;el('flagged').textContent=flagged?'Show all':'Flagged only';render()};document.querySelectorAll('[data-sort]').forEach(e=>e.onclick=()=>{direction=sort===e.dataset.sort?-direction:-1;sort=e.dataset.sort;render()});render();
</script></html>`;
}
