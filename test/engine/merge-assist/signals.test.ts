import { describe, expect, it } from "vitest";
import { candidateSchema, generalizeSignal, mineSignals, signalHits, signalSchema } from "../../../src/engine/index.js";

describe("signalHits", () => {
  it("names the signals that occur in the text, matched case-sensitively as plain substrings", () => {
    const text = "export default function handler() {\n  console.warn(\"x\");\n}\n";
    expect(signalHits(["console.", "export default", "Console.", "as default", "(.*)"], text)).toEqual(["console.", "export default"]);
    expect(signalHits([], text)).toEqual([]);
  });
});

describe("signalSchema", () => {
  it("accepts a one-line fragment of 2 to 40 characters after trimming", () => {
    expect(signalSchema.parse("  ?? ")).toBe("??");
    expect(signalSchema.parse("s".repeat(40))).toBe("s".repeat(40));
  });

  it.each([["a newline", "console.\nlog"], ["a carriage return", "console.\rlog"], ["one character", " ( "], ["41 characters", "s".repeat(41)]])("rejects %s", (_label, signal) => {
    expect(signalSchema.safeParse(signal).success).toBe(false);
  });
});

describe("mineSignals", () => {
  const before = [
    "import axios from \"axios\";",
    "import { format as fmt } from \"./format\";",
    "export default async function load(userId: string) {",
    "  const cache = new Map();",
    "  const { data, status: code } = await axios.get(`/users/${userId}`);",
    "  console.log(\"loaded\", data.items.length, cache.size); // fetch(debug)",
    "  if (code !== 200) throw new HttpError(code);",
    "  return fmt(data).trim();",
    "}",
  ].join("\n");
  const after = [
    "import { client } from \"./client\";",
    "export async function load(userId: string) {",
    "  const { data } = await client.get(`/users/${userId}`);",
    "  logger.info(\"loaded\");",
    "  return data;",
    "}",
  ].join("\n");

  it("finds the call heads, chains and keyword phrases the correction removed, in order", () => {
    expect(mineSignals({ before, after })).toEqual(["export default", "new Map(", "Map(", "console.log(", "throw new", "throw", "new HttpError(", "HttpError("]);
  });

  it("leaves out names the example declares, comments, string contents and what the corrected code keeps", () => {
    const mined = mineSignals({ before, after });
    // axios and fmt are import bindings, load a function, data, code and cache variables, userId a parameter.
    for (const left of ["axios.get(", "await axios.get(", "fmt(", "load(", "data.items.length", "cache.size", "fetch(", "users"]) expect(mined).not.toContain(left);
    expect(mineSignals({ before: "const total = sum(items);", after: "" })).toEqual(["sum("]);
    expect(mineSignals({ before: "logger.info(x); process.env.API_URL", after: "logger.info(x);" })).toEqual(["process.env.API_URL"]);
  });

  it("finds tagged-template heads and await calls", () => {
    expect(mineSignals({ before: "const q = sql`select 1`;\nawait fetch(url);", after: "" })).toEqual(["sql`", "await fetch(", "fetch("]);
  });
});

describe("generalizeSignal", () => {
  it.each([
    ["console.log(", ["console.log(", "console.log", "console."]],
    ["toLocaleDateString()", ["toLocaleDateString()", "toLocaleDateString"]],
    ["process.env.API_URL", ["process.env.API_URL", "process.env.", "process.env"]],
    ["throw new Error(", ["throw new Error(", "throw new Error", "throw"]],
    ["export default", ["export default", "export"]],
    ["await client.get(", ["await client.get(", "await client.get", "await client."]],
    ["sql`", ["sql`"]],
    ["html`", ["html`", "html"]],
    ["a.b(", ["a.b("]],
    ["console.", ["console."]],
  ])("offers %s and its boundary prefixes, most general last", (signal, forms) => {
    expect(generalizeSignal(signal)).toEqual(forms);
  });
});

describe("candidate signals", () => {
  const candidate = {
    format: "rerail.candidate/v1",
    id: "candidate:log-through-logger-3f9a1c",
    decision: "Log through logger, not console.",
    generality: "repository",
    scope: ["src/**"],
    author: "jbarnes",
    createdAt: "2026-11-03T14:02:11Z",
    state: "active",
    supersedes: null,
  };

  it("are optional and validated as signals", () => {
    expect(candidateSchema.safeParse(candidate).success).toBe(true);
    expect(candidateSchema.parse({ ...candidate, signals: ["console.", " logger.debug "] }).signals).toEqual(["console.", "logger.debug"]);
  });

  it.each([
    ["a signal with a newline", ["console.\nlog"]],
    ["an over-long signal", ["s".repeat(41)]],
    ["nine signals", Array.from({ length: 9 }, (_, index) => `retry${index}(`)],
  ])("refuse %s", (_label, signals) => {
    expect(candidateSchema.safeParse({ ...candidate, signals }).success).toBe(false);
  });
});
