#!/usr/bin/env node
// Structure gate: the repository layout's Definition of Done (docs/repository-structure.md).
//
// Runs every metric module in scripts/check/check-structure/metrics/ (`NN-<slug>.mjs`, exporting
// `{ id, title, target, check(repo) }`; `check` returns `{ actual, findings: [{ path, line?, message }] }`)
// over the tracked files of the checkout, prints one table row per metric, then every finding of each
// failing metric as `path[:line]: message`. A metric passes when it has no findings. Exit status 1 when any
// metric fails.
//
//   node scripts/check/check-structure.mjs [--json] [--only <id,…>] [--root <dir>]
//
// Deterministic: its only inputs are `git ls-files` and file contents.

import { readdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { loadRepo } from "./check-structure/repo.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const METRICS = join(HERE, "check-structure", "metrics");

/** The metric modules, in id order; with `only`, just those ids (the others are never imported). */
export async function loadMetrics(only = null) {
  const files = readdirSync(METRICS)
    .filter((file) => /^\d{2}-[a-z0-9-]+\.mjs$/.test(file))
    .sort();
  const ids = files.map((file) => Number(file.slice(0, 2)));
  const unknown = only === null ? [] : [...only].filter((id) => !ids.includes(id));
  if (unknown.length > 0) throw new Error(`no metric ${unknown.join(", ")}`);
  const metrics = [];
  for (const file of files) {
    if (only !== null && !only.has(Number(file.slice(0, 2)))) continue;
    const module = await import(pathToFileURL(join(METRICS, file)).href);
    if (module.id !== Number(file.slice(0, 2))) throw new Error(`${file}: exports id ${module.id}`);
    metrics.push({ id: module.id, title: module.title, target: module.target, check: module.check });
  }
  return metrics;
}

export async function runMetrics(repo, metrics) {
  const results = [];
  for (const metric of metrics) {
    const { actual, findings } = await metric.check(repo);
    results.push({
      id: metric.id,
      title: metric.title,
      target: metric.target,
      actual,
      pass: findings.length === 0,
      findings,
    });
  }
  return results;
}

function parseArgs(argv) {
  const options = { json: false, only: null, root: resolve(HERE, "..", "..") };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--json") options.json = true;
    else if (arg === "--only" || arg.startsWith("--only=")) {
      const value = arg === "--only" ? argv[(i += 1)] : arg.slice("--only=".length);
      if (!value) throw new Error("--only needs metric ids, e.g. --only 1,12,25");
      options.only ??= new Set();
      for (const part of value.split(",")) {
        const number = Number(part.trim().replace(/^#/, ""));
        if (!Number.isInteger(number)) throw new Error(`--only: not a metric id: ${part}`);
        options.only.add(number);
      }
    } else if (arg === "--root") options.root = resolve(argv[(i += 1)] ?? ".");
    else throw new Error(`unknown argument: ${arg}`);
  }
  return options;
}

function location(item) {
  return item.line === undefined ? item.path : `${item.path}:${item.line}`;
}

export function formatTable(results) {
  const rows = [["#", "Metric", "Target", "Actual", "Result"]];
  for (const result of results) {
    rows.push([String(result.id), result.title.replace(/`/g, ""), result.target, result.actual, result.pass ? "PASS" : "FAIL"]);
  }
  const widths = rows[0].map((_, column) => Math.max(...rows.map((row) => row[column].length)));
  const line = (row) => row.map((cell, column) => cell.padEnd(widths[column])).join("  ").trimEnd();
  const out = [line(rows[0]), widths.map((width) => "-".repeat(width)).join("  "), ...rows.slice(1).map(line)];
  const failing = results.filter((result) => !result.pass);
  for (const result of failing) {
    out.push("", `#${result.id} ${result.title.replace(/`/g, "")}: ${result.findings.length} finding(s)`);
    for (const item of result.findings) out.push(`${location(item)}: ${item.message}`);
  }
  out.push("", `${results.length - failing.length} pass, ${failing.length} fail`);
  return out.join("\n");
}

async function main() {
  let options;
  let metrics;
  try {
    options = parseArgs(process.argv.slice(2));
    metrics = await loadMetrics(options.only);
  } catch (error) {
    process.stderr.write(`check-structure: ${error.message}\n`);
    process.exit(2);
  }
  const results = await runMetrics(loadRepo(options.root), metrics);
  process.stdout.write(options.json ? `${JSON.stringify(results, null, 2)}\n` : `${formatTable(results)}\n`);
  process.exitCode = results.every((result) => result.pass) ? 0 : 1;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) await main();
