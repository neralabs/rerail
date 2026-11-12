// Demo: the steer an agent gets for the same unsaved edit, from the session path (steer mode, ai-output scope):
// the runtime's router with a steer delivery that prints what it is asked to send.
import { mkdtempSync, readFileSync, realpathSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const [engine, given] = process.argv.slice(2);
// The router's session folder is physical (`/tmp` is `/private/tmp` on macOS).
const checkout = realpathSync(given);
process.env.RERAIL_CONTENT_STORE_PATH = join(mkdtempSync(join(tmpdir(), "rerail-steer-")), "content.sqlite");
const { CheckoutCatalogs } = await import(join(engine, "dist/engine/code-rule/code-rule-dsl-catalog.js"));
const { CheckoutOwners } = await import(join(engine, "dist/engine/catalog/checkout-owner.js"));
const { CheckoutCodeRuleRouter } = await import(join(engine, "dist/engine/code-rule/checkout-code-rule-router.js"));

const path = "l6500_evidence/capability_store.py";
const before = readFileSync(join(checkout, path), "utf8");
const PAIR = `    @legacy_sync
    def is_token_revoked(self, txn: ReadTransaction, token_id: SysId) -> bool:
        _legacy_delegate_stub()

    @legacy_async
    async def is_token_revoked_async(self, txn: AsyncReadTransaction, token_id: SysId) -> bool:
        _legacy_delegate_stub_async()
`;
const EDIT = `    def is_token_revoked(self, txn: ReadTransaction, token_id: SysId) -> bool:
        return self.is_token_revoked_lazy(txn, token_id).resolve()

    async def is_token_revoked_async(self, txn: AsyncReadTransaction, token_id: SysId) -> bool:
        return self.is_token_revoked(txn, token_id)
`;
if (!before.includes(PAIR)) throw new Error("pair not found");

const steers = [];
const catalogs = new CheckoutCatalogs();
const router = new CheckoutCodeRuleRouter({
  sessionRoot: checkout, startingCheckout: checkout, owners: new CheckoutOwners(() => catalogs.recordedCheckoutsVersion()), catalogs,
  steerDelivery: { requestSteer: async (_session, _delivery, content) => { steers.push(content); } },
  audits: { start: input => { console.error(`audit start ${input.scope}/${input.mode} ${input.paths}`); return "a"; }, finish(_id, r) { console.error(`audit ${r.status}: ${(r.violations ?? []).map(v => `${v.span?.startLine + 1} ${v.message}`).join("; ")} ${(r.failures ?? []).map(f => f.message).join("; ")}`); }, progress() {}, interrupt() {} },
  scope: process.argv[4] ?? "ai-output", mode: "steer", followGitignore: catalogs.followsGitignore(checkout),
});
await router.openWorkspace("demo");
const response = await router.auditGeneratedEdits("demo", {
  revision: 1, checkpoints: ["edit"],
  edits: [{ path, language: "python", previousText: before, generatedText: before.replace(PAIR, EDIT) }],
});
await router.settleGeneratedEdits();
await router.settle();
console.error(`response: ${response.disposition}, ${response.violations.length} violation(s); steers sent: ${steers.length}`);
await router.close();
catalogs.close();
process.stdout.write(steers.join("\n\n---\n\n") + "\n");
