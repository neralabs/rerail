// Release guard: a Linux engine must load on every distribution docs/installation.md supports (glibc 2.28, Node 24's own
// floor). package.yml links it with napi-rs's cross toolchain, against glibc 2.17 and GCC 4.8.5's libstdc++. A shared
// library may leave a symbol neither defines undefined, without a version, for whatever library the machine loading it
// happens to have: glibc's `gettid` (2.30) was one, and `-z defs` is not on for the engine. This fails the build when
// the engine needs a glibc newer than the floor, or leaves any symbol undefined but Node-API's, which Node defines.
import { execFileSync } from "node:child_process";

const FLOOR = [2, 28];
const NODE_API = /^(?:napi_|node_api_)/u;

const files = process.argv.slice(2);
if (files.length === 0) {
  console.error("Usage: node scripts/check/check-linux-engine.mjs <engine.node>...");
  process.exit(2);
}

/** Version numbers of a `2.28`-style string, compared part by part. */
const parts = version => version.split(".").map(Number);
const newer = (left, right) => {
  for (let index = 0; index < Math.max(left.length, right.length); index += 1) {
    const difference = (left[index] ?? 0) - (right[index] ?? 0);
    if (difference !== 0) return difference > 0;
  }
  return false;
};

let failed = false;
for (const file of files) {
  // `readelf --dyn-syms -W`: `Num: Value Size Type Bind Vis Ndx Name`, a versioned name as `name@GLIBC_2.2.5 (2)`.
  const symbols = execFileSync("readelf", ["--dyn-syms", "--wide", file], { encoding: "utf8", maxBuffer: 64 * 1024 * 1024 })
    .split("\n")
    .map(line => line.trim().split(/\s+/u))
    .filter(columns => columns.length >= 8 && columns[6] === "UND" && columns[7] !== "")
    .map(columns => ({ bind: columns[4], name: columns[7] }));
  let newest = [0];
  const unresolved = [];
  for (const { bind, name } of symbols) {
    const [symbol, version] = name.split("@");
    const glibc = version?.match(/^GLIBC_([\d.]+)$/u);
    if (glibc && newer(parts(glibc[1]), newest)) newest = parts(glibc[1]);
    if (version === undefined && bind !== "WEAK" && !NODE_API.test(symbol)) unresolved.push(symbol);
  }
  console.log(`${file}: newest glibc symbol version GLIBC_${newest.join(".")}`);
  if (newer(newest, FLOOR)) {
    console.error(`${file} needs GLIBC_${newest.join(".")}, newer than glibc ${FLOOR.join(".")}.`);
    failed = true;
  }
  if (unresolved.length > 0) {
    console.error(`${file} leaves symbols the link did not resolve to the machine loading it: ${unresolved.sort().join(", ")}.`);
    failed = true;
  } else {
    console.log(`${file}: every undefined symbol but Node-API's is resolved`);
  }
}
process.exitCode = failed ? 1 : 0;
