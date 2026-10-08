// Writes natural.expected.json: naturalCompare(a, b) from the natural-compare package for every ordered pair of the
// strings below, as is and with both sides lower-cased (ESLint sort-keys with natural: true and caseSensitive: false).
// Strings are UTF-16 code units. Run offline, not in CI:
//   node generate_natural.mjs <node_modules with natural-compare@1.4.0>
import { writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { join, resolve } from "node:path";

const require = createRequire(join(resolve(process.argv[2]), "noop.js"));
const naturalCompare = require("natural-compare");
const version = require("natural-compare/package.json").version;

const STRINGS = [
  "", "a", "A", "b", "B", "z", "Z", "_", "~", "[", "`", "{", "-", ".", "/", ":", "@", " ",
  "a1", "a2", "a10", "a01", "a001", "a1b", "a10b", "a 1", "a  1", "a-1", "a_1", "a.1", "a/1",
  "1", "2", "10", "02", "010", "0", "00", "1.5", "1.10", "-1", "-2", "1e3",
  "file9.txt", "file10.txt", "File10.txt", "img12.png", "img10.png", "IMG11.png",
  "123456789012345678901234567890", "123456789012345678901234567891", "9".repeat(400), "9".repeat(401),
  "é", "É", "ä", "😀", "\uD800", "ǅ", "ß", "İ",
];

const units = text => Array.from({ length: text.length }, (_, index) => text.charCodeAt(index));
const compare = STRINGS.map(a => STRINGS.map(b => [naturalCompare(a, b), naturalCompare(a.toLowerCase(), b.toLowerCase())]));
const lines = compare.map(row => JSON.stringify(row));
writeFileSync(
  join(import.meta.dirname, "natural.expected.json"),
  `{"naturalCompare": ${JSON.stringify(version)}, "strings": ${JSON.stringify(STRINGS.map(units))}, "compare": [\n${lines.join(",\n")}\n]}\n`,
);
