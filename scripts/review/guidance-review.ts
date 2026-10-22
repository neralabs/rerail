/**
 * `just rerail-guidance-review`: review every rule pack rule's steer guidance in the browser, approving it or asking
 * for changes, with comments. Serves `guidance-review.html` on localhost; the page reads the rules from
 * `.rerail/guidance-review/data.json` (written by `guidance-review-data.ts`, here when missing or with --refresh) and
 * saves each decision to `.rerail/guidance-review/decisions.json` as it is made.
 *
 *   node --import tsx scripts/review/guidance-review.ts [--refresh] [--port <n>]
 */
import { existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { createServer, type IncomingMessage, type ServerResponse } from "node:http";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";
import { z } from "zod";
import { DEFAULT_OUT, reviewData } from "./guidance-review-data.js";

const HERE = dirname(fileURLToPath(import.meta.url));
const PAGE = join(HERE, "guidance-review.html");
const DATA = DEFAULT_OUT;
const DECISIONS = join(dirname(DATA), "decisions.json");

const decisionSchema = z.strictObject({
  status: z.enum(["approved", "changes", "pending"]),
  comment: z.string(),
  /** Preset feedback picked above the comment (the page's FEEDBACK list). */
  tags: z.array(z.string()).default([]),
  /** Comments on single cases, by case id. */
  cases: z.record(z.string(), z.string()),
  /** The rule's digest when it was decided; a different digest now means its wording or guidance changed since. */
  digest: z.string(),
  at: z.string(),
});
const decisionsSchema = z.strictObject({ version: z.literal(1), rules: z.record(z.string(), decisionSchema) });
type Decisions = z.infer<typeof decisionsSchema>;

function readDecisions(): Decisions {
  if (!existsSync(DECISIONS)) return { version: 1, rules: {} };
  return decisionsSchema.parse(JSON.parse(readFileSync(DECISIONS, "utf8")));
}

/** Written whole through a temporary file, so a crash never leaves half a file. */
function writeDecisions(decisions: Decisions): void {
  mkdirSync(dirname(DECISIONS), { recursive: true });
  const sorted = { version: 1, rules: Object.fromEntries(Object.entries(decisions.rules).sort(([a], [b]) => a.localeCompare(b))) };
  writeFileSync(`${DECISIONS}.tmp`, `${JSON.stringify(sorted, null, 2)}\n`);
  renameSync(`${DECISIONS}.tmp`, DECISIONS);
}

async function body(request: IncomingMessage): Promise<string> {
  const chunks: Buffer[] = [];
  for await (const chunk of request) chunks.push(chunk as Buffer);
  return Buffer.concat(chunks).toString("utf8");
}

function send(response: ServerResponse, status: number, type: string, text: string): void {
  response.writeHead(status, { "content-type": type, "cache-control": "no-store" });
  response.end(text);
}

async function handle(request: IncomingMessage, response: ServerResponse): Promise<void> {
  const url = new URL(request.url ?? "/", "http://localhost");
  if (request.method === "GET" && url.pathname === "/") return send(response, 200, "text/html; charset=utf-8", readFileSync(PAGE, "utf8"));
  if (request.method === "GET" && url.pathname === "/data.json") return send(response, 200, "application/json", readFileSync(DATA, "utf8"));
  if (request.method === "GET" && url.pathname === "/decisions.json") return send(response, 200, "application/json", JSON.stringify(readDecisions()));
  const rule = /^\/decisions\/([^/]+)$/u.exec(url.pathname);
  if (request.method === "PUT" && rule !== null) {
    const decision = decisionSchema.parse(JSON.parse(await body(request)));
    const decisions = readDecisions();
    const id = decodeURIComponent(rule[1] ?? "");
    if (decision.status === "pending" && decision.comment.trim() === "" && decision.tags.length === 0 && Object.values(decision.cases).every(text => text.trim() === "")) delete decisions.rules[id];
    else decisions.rules[id] = decision;
    writeDecisions(decisions);
    return send(response, 200, "application/json", JSON.stringify({ ok: true }));
  }
  send(response, 404, "text/plain", "not found");
}

async function main(): Promise<void> {
  const { values } = parseArgs({ options: { refresh: { type: "boolean", default: false }, port: { type: "string", default: "4799" } } });
  if (values.refresh || !existsSync(DATA)) {
    process.stderr.write("Running every rule pack's rules on their cases (about two minutes)…\n");
    const data = await reviewData();
    mkdirSync(dirname(DATA), { recursive: true });
    writeFileSync(DATA, `${JSON.stringify(data)}\n`);
  }
  const server = createServer((request, response) => {
    handle(request, response).catch((error: unknown) => send(response, 500, "text/plain", error instanceof Error ? error.message : String(error)));
  });
  server.listen(Number(values.port), "127.0.0.1", () => {
    process.stdout.write(`Guidance review: http://127.0.0.1:${values.port}/\nDecisions are saved to ${DECISIONS}\n`);
  });
}

await main();
