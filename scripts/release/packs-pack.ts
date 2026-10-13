#!/usr/bin/env node
// The publish gate, then the tarballs: runs the whole pack validation (test/engine/catalog/packs.test.ts, every check
// and every rule) for the named packs, packs each with `npm pack`, and checks that each tarball holds only
// package.json and what the pack's `files` publishes, and no validation material. Writes the tarballs and
// manifest.json (`[{ id, name, version, file }]`) to --out. Needs the built native engine, as the pack tests do.
//
//   node --import tsx scripts/release/packs-pack.ts --out <dir> (<id>… | --all)
import { spawnSync, execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { z } from "zod";
import { PACK_CHECKS, packedFileFailures } from "../../conformance/pack-validation.js";
import { gateFailures, packIds, PACKS, ROOT } from "./packs.mjs";

const reportSchema = z.object({
  testResults: z.array(z.object({ assertionResults: z.array(z.object({ ancestorTitles: z.array(z.string()), title: z.string(), status: z.string() })) })),
});
const packedSchema = z.tuple([z.object({ filename: z.string(), name: z.string(), version: z.string() })]);
const manifestSchema = z.object({ files: z.array(z.string()) });

function fail(lines: string[]): never {
  for (const line of lines) console.error(`::error::${line}`);
  process.exit(1);
}

/** Runs the pack validation for `ids`; its failures, empty when every pack passes. */
function gate(ids: string[]): string[] {
  const scratch = mkdtempSync(join(tmpdir(), "rerail-pack-gate-"));
  const report = join(scratch, "report.json");
  try {
    const vitest = join(ROOT, "node_modules", ".bin", "vitest");
    const run = spawnSync(
      vitest,
      ["run", "test/engine/catalog/packs.test.ts", "-t", `^pack (${ids.join("|")}) `, "--reporter=default", "--reporter=json", `--outputFile.json=${report}`],
      { cwd: ROOT, stdio: "inherit" },
    );
    if (run.error !== undefined) return [`the pack validation did not run: ${run.error.message}`];
    const failures = gateFailures(reportSchema.parse(JSON.parse(readFileSync(report, "utf8"))), ids, PACK_CHECKS.length);
    if (run.status !== 0 && failures.length === 0) failures.push(`the pack validation exited with ${run.status}`);
    return failures;
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
}

/** Packs `id` into `out`, checking what the tarball holds. */
function pack(id: string, out: string): { id: string; name: string; version: string; file: string } {
  const directory = join(PACKS, id);
  const [packed] = packedSchema.parse(JSON.parse(execFileSync("npm", ["pack", "--json", "--ignore-scripts", "--pack-destination", out], { cwd: directory, encoding: "utf8" })));
  const entries = execFileSync("tar", ["-tzf", join(out, packed.filename)], { encoding: "utf8" })
    .split("\n")
    .filter(entry => entry !== "" && !entry.endsWith("/"))
    .map(entry => entry.replace(/^package\//u, ""));
  const { files } = manifestSchema.parse(JSON.parse(readFileSync(join(directory, "package.json"), "utf8")));
  const failures = packedFileFailures(files, entries);
  if (failures.length > 0) fail(failures.map(failure => `${id} tarball: ${failure}`));
  console.log(`packed ${packed.name}@${packed.version}: ${entries.length} files`);
  return { id, name: packed.name, version: packed.version, file: packed.filename };
}

const args = process.argv.slice(2);
const outAt = args.indexOf("--out");
const outArgument = outAt === -1 ? undefined : args[outAt + 1];
if (outArgument === undefined) fail(["usage: packs-pack.ts --out <dir> (<id>… | --all)"]);
const out = resolve(outArgument);
const ids = packIds(args.filter((_, index) => index !== outAt && index !== outAt + 1));
if (ids.length === 0) fail(["no packs named"]);

const failures = gate(ids);
if (failures.length > 0) fail(failures);
mkdirSync(out, { recursive: true });
writeFileSync(join(out, "manifest.json"), `${JSON.stringify(ids.map(id => pack(id, out)), null, 2)}\n`);
