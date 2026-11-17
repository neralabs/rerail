#!/usr/bin/env node
// The Rust files scripts/check/rustfmt-files.txt lists are formatted as rustfmt (edition 2024) lays them out. The list
// holds main's own files and those written for this repository since; the engine line's files keep their author's
// layout. `cargo fmt --all` cannot draw that line: the workspace's crates mount native/src through `#[path]` modules,
// and rustfmt follows every `mod` into the files it declares. Each listed file goes to rustfmt on stdin, where rustfmt
// formats that file alone. A listed file that is gone fails too: take it off the list.
//
//   node scripts/check/check-rustfmt.mjs           (npm run check:native runs it before clippy)
//   node scripts/check/check-rustfmt.mjs --write   formats the listed files in place
import { spawnSync } from "node:child_process";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
export const LIST = "scripts/check/rustfmt-files.txt";

/** The paths a list's text names: a path per line; blank lines and `#` comments are skipped. */
export function listedFiles(text) {
  return text.split("\n").map((line) => line.trim()).filter((line) => line !== "" && !line.startsWith("#"));
}

/** `source` as rustfmt (edition 2024) formats it, run in `root` so the toolchain rust-toolchain.toml pins is used. */
export function rustfmt(root) {
  return (source, path) => {
    const run = spawnSync("rustfmt", ["--edition", "2024", "--emit", "stdout"], {
      cwd: root, input: source, encoding: "utf8", maxBuffer: 1 << 28,
    });
    if (run.error !== undefined) throw run.error;
    if (run.status !== 0) throw new Error(`rustfmt failed on ${path}:\n${run.stderr}`);
    return run.stdout;
  };
}

/** Each listed file under `root` that is gone or not as `format` lays it out, as `{ path, problem, formatted }`. */
export function rustfmtProblems(root, files, format = rustfmt(root)) {
  return files.flatMap((path) => {
    const file = join(root, path);
    if (!existsSync(file)) return [{ path, problem: `is listed in ${LIST} but gone: take it off the list` }];
    const source = readFileSync(file, "utf8");
    const formatted = format(source, path);
    return formatted === source ? [] : [{ path, problem: "is not formatted: node scripts/check/check-rustfmt.mjs --write", formatted }];
  });
}

if (process.argv[1] !== undefined && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const write = process.argv.includes("--write");
  const files = listedFiles(readFileSync(join(ROOT, LIST), "utf8"));
  const problems = rustfmtProblems(ROOT, files);
  for (const { path, problem, formatted } of problems) {
    if (write && formatted !== undefined) writeFileSync(join(ROOT, path), formatted);
    else console.error(`${path} ${problem}`);
  }
  const left = write ? problems.filter(({ formatted }) => formatted === undefined) : problems;
  console.log(`check-rustfmt: ${files.length} file(s) checked, ${problems.length - left.length} formatted, ${left.length} problem(s)`);
  process.exit(left.length === 0 ? 0 : 1);
}
