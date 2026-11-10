// Demo: an editor session on l6500_evidence/capability_store.py in the data-layer repository at 2b1504ad, driven over LSP.
// Unsaved edits rewrite the `is_token_revoked` sync/async pair; Rerail's diagnostics are printed after each step.
import { spawn } from "node:child_process";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

const [engine, checkout] = process.argv.slice(2);
const file = join(checkout, "l6500_evidence/capability_store.py");
const uri = pathToFileURL(file).href;
const original = readFileSync(file, "utf8");

const server = spawn(process.execPath, [join(engine, "dist/cli/cli.js"), "lsp"], { cwd: checkout, stdio: ["pipe", "pipe", "inherit"] });
let buffer = Buffer.alloc(0);
const waiters = [];
server.stdout.on("data", chunk => {
  buffer = Buffer.concat([buffer, chunk]);
  for (;;) {
    const header = buffer.indexOf("\r\n\r\n");
    if (header < 0) return;
    const length = Number(/Content-Length: (\d+)/i.exec(buffer.subarray(0, header).toString())[1]);
    if (buffer.length < header + 4 + length) return;
    const message = JSON.parse(buffer.subarray(header + 4, header + 4 + length).toString());
    buffer = buffer.subarray(header + 4 + length);
    if (message.method === "window/logMessage") console.log(`  [server] ${message.params.message.slice(0, 200)}`);
    for (const waiter of [...waiters]) if (waiter.match(message)) { waiters.splice(waiters.indexOf(waiter), 1); waiter.resolve(message); }
  }
});
const send = message => {
  const body = JSON.stringify({ jsonrpc: "2.0", ...message });
  server.stdin.write(`Content-Length: ${Buffer.byteLength(body)}\r\n\r\n${body}`);
};
const next = (match, ms = 900_000) => new Promise((resolve, reject) => {
  waiters.push({ match, resolve });
  setTimeout(() => reject(new Error("timed out")), ms).unref();
});
const published = version => next(m => m.method === "textDocument/publishDiagnostics" && m.params.uri === uri && m.params.version === version);
const SEVERITY = { 1: "error", 2: "warning", 3: "info", 4: "hint" };

let text = original;
const at = offset => {
  const before = text.slice(0, offset).split("\n");
  return { line: before.length - 1, character: before.at(-1).length };
};
function show(step, message, ms, region) {
  const diagnostics = message.params.diagnostics;
  const lines = text.split("\n");
  const inRegion = diagnostics.filter(d => region === undefined || (d.range.start.line >= region.start && d.range.start.line <= region.end));
  console.log(`\n${step}: ${diagnostics.length} diagnostic(s) in the file, published ${ms} ms after the edit${region ? `; in the edited lines ${region.start + 1}-${region.end + 1}:` : ""}`);
  for (const d of inRegion) {
    console.log(`  ${SEVERITY[d.severity]} ${d.range.start.line + 1}:${d.range.start.character + 1} [${d.code}] ${d.message}`);
    console.log(`      ${lines[d.range.start.line].trim()}`);
  }
}
const ORIGINAL_PAIR = `    @legacy_sync
    def is_token_revoked(self, txn: ReadTransaction, token_id: SysId) -> bool:
        _legacy_delegate_stub()

    @legacy_async
    async def is_token_revoked_async(self, txn: AsyncReadTransaction, token_id: SysId) -> bool:
        _legacy_delegate_stub_async()
`;
let current = ORIGINAL_PAIR;
let version = 1;
async function replacePair(step, replacement) {
  const start = text.indexOf(current);
  if (start < 0) throw new Error("pair not found");
  const range = { start: at(start), end: at(start + current.length) };
  text = text.slice(0, start) + replacement + text.slice(start + current.length);
  current = replacement;
  version += 1;
  const sent = performance.now();
  send({ method: "textDocument/didChange", params: { textDocument: { uri, version }, contentChanges: [{ range, text: replacement }] } });
  const message = await published(version);
  const first = at(start).line;
  show(step, message, Math.round(performance.now() - sent), { start: first, end: first + replacement.split("\n").length - 2 });
}

send({ id: 1, method: "initialize", params: { processId: process.pid, rootUri: pathToFileURL(checkout).href, capabilities: {} } });
await next(m => m.id === 1);
send({ method: "initialized", params: {} });
let sent = performance.now();
send({ method: "textDocument/didOpen", params: { textDocument: { uri, languageId: "python", version, text } } });
show("open, as committed", await published(version), Math.round(performance.now() - sent));

await replacePair("edit 1, the pair runs its own eager queries and skips is_token_revoked_lazy", `    def is_token_revoked(self, txn: ReadTransaction, token_id: SysId) -> bool:
        return self._revocation_lookup_query(txn, token_id).exists()

    async def is_token_revoked_async(self, txn: AsyncReadTransaction, token_id: SysId) -> bool:
        return await self._revocation_lookup_query_async(txn, token_id).exists_async()
`);
await replacePair("edit 2, the async method calls the sync one with no await", `    def is_token_revoked(self, txn: ReadTransaction, token_id: SysId) -> bool:
        return self.is_token_revoked_lazy(txn, token_id).resolve()

    async def is_token_revoked_async(self, txn: AsyncReadTransaction, token_id: SysId) -> bool:
        return self.is_token_revoked(txn, token_id)
`);
await replacePair("edit 3, both resolve through the shared lazy family", `    def is_token_revoked(self, txn: ReadTransaction, token_id: SysId) -> bool:
        return self.is_token_revoked_lazy(txn, token_id).resolve()

    async def is_token_revoked_async(self, txn: AsyncReadTransaction, token_id: SysId) -> bool:
        return await self.is_token_revoked_lazy(txn, token_id).resolve_async()
`);

send({ id: 2, method: "shutdown" });
await next(m => m.id === 2);
send({ method: "exit" });
