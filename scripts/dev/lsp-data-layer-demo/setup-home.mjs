// Demo setup: an isolated Rerail home where the scratch data-layer checkout is recorded, started and set up,
// with the data-layer candidate rules from Jeremy's branch as its worktree rules, approved so they run at their own mode.
import { copyFileSync, mkdirSync, readFileSync, readdirSync, realpathSync } from "node:fs";
import { join } from "node:path";

const [engine, checkout, rulesDir] = process.argv.slice(2);
const { openRerailDatabase, recordCheckout, setUpRepository } = await import(join(engine, "dist/engine/index.js"));
const { setRepositoryStarted } = await import(join(engine, "dist/runtime/index.js"));
const { approve } = await import(join(engine, "dist/engine/catalog/rule-approval.js"));
const { ruleDocumentDigest } = await import(join(engine, "dist/engine/stable-json.js"));
const { worktreeRuleCatalog } = await import(join(engine, "dist/engine/catalog/worktree-pack.js"));
const { loadRuleCatalog } = await import(join(engine, "dist/engine/catalog/rule-catalog.js"));

const target = join(checkout, ".rerail", "rules");
mkdirSync(target, { recursive: true });
const database = openRerailDatabase(join(process.env.RERAIL_HOME, "rerail.sqlite"));
try {
  const { repositoryKey } = recordCheckout(database, checkout);
  setRepositoryStarted(database, repositoryKey, true);
  setUpRepository(database, repositoryKey, { packs: [] });
  const files = readdirSync(rulesDir).filter(name => name.endsWith(".json"));
  for (const file of files) copyFileSync(join(rulesDir, file), join(target, file));
  // Approval is of the document as the catalog reads it, so its digest comes from there.
  const catalog = worktreeRuleCatalog(loadRuleCatalog(), realpathSync(checkout));
  for (const file of files) {
    const { id, mode } = JSON.parse(readFileSync(join(rulesDir, file), "utf8"));
    const document = catalog.documents.get(id);
    if (document === undefined) throw new Error(`${id} is not in the checkout's worktree rules`);
    approve(database, { kind: "rule", id, digest: ruleDocumentDigest(document) });
    console.log(`worktree rule ${id} (${mode}), approved`);
  }
} finally {
  database.$client.close();
}
