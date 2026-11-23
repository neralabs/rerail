/**
 * `rerail rules approve`: list the rules and packs whose content waits for a person's approval here, or approve one
 * (`rule-approval.ts`), printing what it approves first. Until approved, such a rule runs in Log only. Approving is a
 * person's: in a coding agent's session (`agent-session.ts`) it lists what waits, and refuses to approve, sending the
 * person to each one's page in the dashboard, as the API refuses an MCP caller.
 */
import { resolve } from "node:path";
import { eq } from "drizzle-orm";
import { parseArguments } from "../daemon/daemon.js";
import { repositoryIdentityLocation } from "../engine/catalog/repository-identity.js";
import { approvalKey, approve, currentSubject, packSubject, readApprovals, ruleSubject, type RuleApprovalSubject } from "../engine/catalog/rule-approval.js";
import { activeCoreVersion, loadRuleCatalog } from "../engine/catalog/rule-catalog.js";
import { declaredPackCatalog, PacksFileError, readPacksFile, repositoryPackDirectory } from "../engine/catalog/packs-file/packs-file.js";
import { RERAIL_PUBLISHER } from "../engine/catalog/rule-packs.js";
import { isWorktreePack, WORKTREE_PACK_ID, WORKTREE_RULES_FOLDER, worktreeRuleCatalog } from "../engine/catalog/worktree-pack.js";
import { experimentEnabled } from "../engine/experimental-features.js";
import { defaultRerailDatabasePath, openRerailDatabase, type RerailDatabase } from "../engine/storage/database.js";
import { libraryRules } from "../engine/storage/schema.js";
import { ruleDocumentDigest } from "../engine/stable-json.js";
import { clean } from "../harness/setup-screens.js";
import { agentRunsText, agentSession, type AgentSession } from "../engine/agent-session.js";
import type { RuleCatalog } from "../engine/catalog/repository-model.js";

interface Waiting {
  readonly subject: RuleApprovalSubject;
  /** Where it comes from, for the person: "created by you", "this worktree's .rerail/rules", "by @acme". */
  readonly from: string;
}

/**
 * Everything waiting for approval that runs in the checkout `catalog` is for: library rules, its own rules, packs; and
 * the packs `declared` reads from the checkout's `.rerail/packs.json`.
 */
function waiting(db: RerailDatabase, catalog: RuleCatalog, declared: RuleCatalog | undefined): Waiting[] {
  const approvals = readApprovals(db);
  const library = db.select({ ruleId: libraryRules.ruleId, payloadHash: libraryRules.payloadHash }).from(libraryRules).all()
    .map(row => ({ subject: ruleSubject(row.ruleId, row.payloadHash), from: "created by you" }));
  const worktree = catalog.packs.get(WORKTREE_PACK_ID);
  const own = worktree === undefined || !isWorktreePack(worktree) ? [] : worktree.rules.flatMap(rule => {
    const document = catalog.documents.get(rule.id);
    return document === undefined ? [] : [{ subject: ruleSubject(rule.id, ruleDocumentDigest(document)), from: `this worktree's ${WORKTREE_RULES_FOLDER}` }];
  });
  const packs = [catalog, ...declared === undefined ? [] : [declared]].flatMap(owner => [...owner.packs.values()].map(pack => ({ owner, pack }))).flatMap(({ owner, pack }) => {
    const subject = packSubject(owner, pack);
    if (subject === undefined) return [];
    const by = pack.publisher === RERAIL_PUBLISHER ? "" : `by ${pack.publisher}`;
    const from = pack.localSource === undefined ? by : [by, `from the ${pack.localSource.kind} ${pack.localSource.path}`].filter(Boolean).join(", ");
    return [{ subject: { ...subject, id: `${subject.id}@${pack.version ?? "?"}` }, approvalSubject: subject, from }];
  });
  return [
    ...[...library, ...own].filter(entry => !approvals.has(approvalKey(entry.subject))),
    ...packs.filter((entry, index) => !approvals.has(approvalKey(entry.approvalSubject))
      && packs.findIndex(other => approvalKey(other.approvalSubject) === approvalKey(entry.approvalSubject)) === index).map(({ subject, from }) => ({ subject, from })),
  ];
}

/** What waits here, and how a person approves it: on its page in the dashboard when a coding agent's session asks. */
function list(db: RerailDatabase, catalog: RuleCatalog, declared: RuleCatalog | undefined, write: (text: string) => void, session: AgentSession | undefined): void {
  const entries = waiting(db, catalog, declared);
  if (entries.length === 0) {
    write("● Nothing is waiting for approval here.\n");
    return;
  }
  write([
    "Waiting for approval; each runs in Log, and steers no one, until a person approves it:",
    ...entries.map(({ subject, from }) => `  ${subject.kind} ${clean(subject.id)}${from === "" ? "" : `  (${clean(from)})`}`),
    session === undefined
      ? "Read each before approving it. Approve one with: rerail rules approve <id or package>"
      : "A person reads each and approves it on its page in the dashboard, which rerail manage opens.",
  ].join("\n") + "\n");
}

/**
 * `value` as indented JSON, with every invisible character (format, private-use and unassigned code points: zero-width
 * spaces, bidi controls, tag characters) written as its escape, so the person reads exactly what the agent will.
 */
function visibleJson(value: unknown): string {
  return JSON.stringify(value, null, 2).replace(/[\p{Cf}\p{Co}\p{Cn}]/gu, character => {
    const code = character.codePointAt(0)!;
    return code > 0xffff ? `\\u{${code.toString(16)}}` : `\\u${code.toString(16).padStart(4, "0")}`;
  });
}

/** What approving `subject` approves, written before it is approved: a rule's content in full, a pack's rules by id. */
function approvalText(db: RerailDatabase, catalog: RuleCatalog, subject: RuleApprovalSubject): string {
  if (subject.kind === "pack") {
    const pack = [...catalog.packs.values()].find(candidate => (candidate.packageName ?? candidate.id) === subject.id);
    const ids = pack?.rules.map(rule => clean(rule.id)) ?? [];
    return `● pack ${clean(subject.id)}@${clean(pack?.version ?? "?")} ships ${ids.length} ${ids.length === 1 ? "rule" : "rules"}: ${ids.join(", ")}\n`;
  }
  const document = db.select({ document: libraryRules.document }).from(libraryRules).where(eq(libraryRules.ruleId, subject.id)).get()?.document
    ?? catalog.documents.get(subject.id);
  return `● ${clean(subject.id)}${document === undefined ? "" : ` (${document.kind} rule)`} has this content:\n${visibleJson(document?.rule)}\n`;
}

/**
 * Internal testing: the packs the checkout at `root` keeps in the repository and lists in `.rerail/packs.json`, read
 * from its working tree as a scan of it reads them, so they can be approved without installing them. Undefined while
 * headless scan is off, or when the checkout lists none; a file or pack folder that cannot be read is reported with
 * `write` and left out, so the rest can still be listed and approved.
 */
function declaredRepositoryPacks(root: string, write: (text: string) => void): RuleCatalog | undefined {
  if (!experimentEnabled("HEADLESS_SCAN")) return undefined;
  try {
    const repositoryPacks = (readPacksFile(root) ?? []).flatMap(pack => pack.kind === "repository" ? [pack] : []);
    if (repositoryPacks.length === 0) return undefined;
    return declaredPackCatalog(repositoryPacks.map(pack => {
      const directory = repositoryPackDirectory(root, pack);
      return { pack, directory, checkoutPath: directory };
    }), { coreVersion: activeCoreVersion() });
  } catch (error) {
    if (!(error instanceof PacksFileError)) throw error;
    write(`● ${clean(error.message)}; the packs it lists are left out here.\n`);
    return undefined;
  }
}

/** Why a coding agent's session cannot approve `names`, and where a person does. */
function agentApprovalRefusal(names: readonly string[], session: AgentSession): string {
  const one = names.length === 1;
  return `${agentRunsText("rules approve", session)}. A person reads and approves ${names.map(clean).join(", ")} on ${one ? "its" : "their"} `
    + `page in the dashboard, which rerail manage opens, or with rerail rules approve in a terminal of their own.`;
}

/** `rerail rules approve [<rule id | pack> …] [--repo <dir>]`. */
export function runRuleApproveCommand(
  args: readonly string[],
  usage: () => string,
  io: { stdout?: { write(text: string): unknown } } = {},
  databasePath: string = defaultRerailDatabasePath(),
  env: NodeJS.ProcessEnv = process.env,
): number {
  const write = (text: string): void => { (io.stdout ?? process.stdout).write(text); };
  // The names come first, then the options, as `rules test <draft.json> --repo <dir>` takes them.
  const first = args.findIndex(argument => argument.startsWith("-"));
  const positionals = first === -1 ? [...args] : args.slice(0, first);
  const { options } = parseArguments(first === -1 ? [] : args.slice(first), { flags: [], options: ["--repo"] }, argument => `Unknown rules approve option: ${argument}\n\n${usage()}`);
  const session = agentSession(env);
  if (session !== undefined && positionals.length > 0) throw new Error(agentApprovalRefusal(positionals, session));
  const { workspaceRoot } = repositoryIdentityLocation(resolve(options.get("--repo") ?? process.cwd()));
  const catalog = worktreeRuleCatalog(loadRuleCatalog(), workspaceRoot);
  const declared = declaredRepositoryPacks(workspaceRoot, write);
  const db = openRerailDatabase(databasePath);
  try {
    if (positionals.length === 0) {
      list(db, catalog, declared, write, session);
      return 0;
    }
    const approvals = readApprovals(db);
    // A name can be both an installed pack and a pack the repository keeps, with other content: each that waits is
    // shown, then approved, so the copy a scan of this checkout runs is never left waiting behind the installed one.
    const subjects = positionals.flatMap(name => {
      const found = [catalog, ...declared === undefined ? [] : [declared]].flatMap(owner => {
        const subject = currentSubject(db, owner, name);
        return subject === undefined ? [] : [{ subject, owner }];
      }).filter((entry, index, all) => all.findIndex(other => approvalKey(other.subject) === approvalKey(entry.subject)) === index);
      if (found.length === 0) throw new Error(`${name} is no rule or pack waiting for approval here: Rerail's own rules and packs need none. \`rerail rules approve\` lists what waits.`);
      return found;
    });
    for (const { subject, owner } of subjects) {
      if (approvals.has(approvalKey(subject))) {
        write(`● ${clean(subject.id)} is already approved as it is.\n`);
        continue;
      }
      write(approvalText(db, owner, subject));
      approve(db, subject);
      write(`● Approved ${subject.kind} ${clean(subject.id)}: it runs in the mode it is set to from the next edit.\n`);
    }
    return 0;
  } finally {
    db.$client.close();
  }
}
