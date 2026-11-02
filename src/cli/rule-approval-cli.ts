/**
 * `rerail rules approve`: list the rules and packs whose content waits for a person's approval here, or approve one
 * (`rule-approval.ts`). Until approved, such a rule runs in Log only. A coding agent runs this only after the person
 * said yes to the content it read back to them (`skills/rerail-create`, "Approve").
 */
import { resolve } from "node:path";
import { parseArguments } from "../daemon/daemon.js";
import { repositoryIdentityLocation } from "../engine/catalog/repository-identity.js";
import { approvalKey, approve, currentSubject, packSubject, readApprovals, ruleSubject, type RuleApprovalSubject } from "../engine/catalog/rule-approval.js";
import { loadRuleCatalog } from "../engine/catalog/rule-catalog.js";
import { RERAIL_PUBLISHER } from "../engine/catalog/rule-packs.js";
import { isWorktreePack, WORKTREE_PACK_ID, WORKTREE_RULES_FOLDER, worktreeRuleCatalog } from "../engine/catalog/worktree-pack.js";
import { defaultRerailDatabasePath, openRerailDatabase, type RerailDatabase } from "../engine/storage/database.js";
import { libraryRules } from "../engine/storage/schema.js";
import { ruleDocumentDigest } from "../engine/stable-json.js";
import { clean } from "../harness/setup-screens.js";
import type { RuleCatalog } from "../engine/catalog/repository-model.js";

interface Waiting {
  readonly subject: RuleApprovalSubject;
  /** Where it comes from, for the person: "created by you", "this worktree's .rerail/rules", "by @acme". */
  readonly from: string;
}

/** Everything waiting for approval that runs in the checkout `catalog` is for: library rules, its own rules, packs. */
function waiting(db: RerailDatabase, catalog: RuleCatalog): Waiting[] {
  const approvals = readApprovals(db);
  const library = db.select({ ruleId: libraryRules.ruleId, payloadHash: libraryRules.payloadHash }).from(libraryRules).all()
    .map(row => ({ subject: ruleSubject(row.ruleId, row.payloadHash), from: "created by you" }));
  const worktree = catalog.packs.get(WORKTREE_PACK_ID);
  const own = worktree === undefined || !isWorktreePack(worktree) ? [] : worktree.rules.flatMap(rule => {
    const document = catalog.documents.get(rule.id);
    return document === undefined ? [] : [{ subject: ruleSubject(rule.id, ruleDocumentDigest(document)), from: `this worktree's ${WORKTREE_RULES_FOLDER}` }];
  });
  const packs = [...catalog.packs.values()].flatMap(pack => {
    const subject = packSubject(catalog, pack);
    if (subject === undefined) return [];
    const by = pack.publisher === RERAIL_PUBLISHER ? "" : `by ${pack.publisher}`;
    const from = pack.localSource === undefined ? by : [by, `from the ${pack.localSource.kind} ${pack.localSource.path}`].filter(Boolean).join(", ");
    return [{ subject: { ...subject, id: `${subject.id}@${pack.version ?? "?"}` }, approvalSubject: subject, from }];
  });
  return [
    ...[...library, ...own].filter(entry => !approvals.has(approvalKey(entry.subject))),
    ...packs.filter(entry => !approvals.has(approvalKey(entry.approvalSubject))).map(({ subject, from }) => ({ subject, from })),
  ];
}

function list(db: RerailDatabase, catalog: RuleCatalog, write: (text: string) => void): void {
  const entries = waiting(db, catalog);
  if (entries.length === 0) {
    write("● Nothing is waiting for approval here.\n");
    return;
  }
  write([
    "Waiting for approval; each runs in Log, and steers no one, until a person approves it:",
    ...entries.map(({ subject, from }) => `  ${subject.kind} ${clean(subject.id)}${from === "" ? "" : `  (${clean(from)})`}`),
    "Read each before approving it. Approve one with: rerail rules approve <id or package>",
  ].join("\n") + "\n");
}

/** `rerail rules approve [<rule id | pack> …] [--repo <dir>]`. */
export function runRuleApproveCommand(
  args: readonly string[],
  usage: () => string,
  io: { stdout?: { write(text: string): unknown } } = {},
  databasePath: string = defaultRerailDatabasePath(),
): number {
  const write = (text: string): void => { (io.stdout ?? process.stdout).write(text); };
  // The names come first, then the options, as `rules test <draft.json> --repo <dir>` takes them.
  const first = args.findIndex(argument => argument.startsWith("-"));
  const positionals = first === -1 ? [...args] : args.slice(0, first);
  const { options } = parseArguments(first === -1 ? [] : args.slice(first), { flags: [], options: ["--repo"] }, argument => `Unknown rules approve option: ${argument}\n\n${usage()}`);
  const { workspaceRoot } = repositoryIdentityLocation(resolve(options.get("--repo") ?? process.cwd()));
  const catalog = worktreeRuleCatalog(loadRuleCatalog(), workspaceRoot);
  const db = openRerailDatabase(databasePath);
  try {
    if (positionals.length === 0) {
      list(db, catalog, write);
      return 0;
    }
    const approvals = readApprovals(db);
    const subjects = positionals.map(name => {
      const subject = currentSubject(db, catalog, name);
      if (subject === undefined) throw new Error(`${name} is no rule or pack waiting for approval here: Rerail's own rules and packs need none. \`rerail rules approve\` lists what waits.`);
      return subject;
    });
    for (const subject of subjects) {
      if (approvals.has(approvalKey(subject))) {
        write(`● ${clean(subject.id)} is already approved as it is.\n`);
        continue;
      }
      approve(db, subject);
      write(`● Approved ${subject.kind} ${clean(subject.id)}: it runs in the mode it is set to from the next edit.\n`);
    }
    return 0;
  } finally {
    db.$client.close();
  }
}
