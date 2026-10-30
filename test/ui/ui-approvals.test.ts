import { execFileSync } from "node:child_process";
import { mkdirSync, realpathSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createRerailApi, DatabaseApiStores, operations } from "../../src/api/index.js";
import { addAttachment, openRerailDatabase, recordCheckout, setUpRepository, upsertLibraryRule } from "../../src/engine/index.js";
import { codeRule } from "../support/rule-configuration-builders.js";
import { send } from "../../ui/api.js";
import {
  approvalView, awaitingCount, awaitingOnPage, awaitingReferences, isRuleChanged, ownerText, pendingNote, waitingTitle,
  type PendingRule,
} from "../../ui/approval-model.js";
import { refresh, setNeeds, type StoredRule } from "../../ui/poll.js";
import { data, setScope, ui } from "../../ui/state.js";
import { currentNeeds, pageKey } from "../../ui/views/index.js";

const pending = (over: Partial<PendingRule> = {}): PendingRule => ({
  kind: "code", owner: "library", id: "mine", packId: null, rule: codeRule("mine", { message: "Use the logger" }),
  revision: 1, contentHash: "h1", copiedFromRuleId: null, approved: null, ...over,
});

describe("a rule waiting for approval, as the page shows it", () => {
  it("shows a new rule's whole content and says it runs nowhere yet", () => {
    const view = approvalView(pending());
    expect(view.runs).toBe("nowhere");
    expect(view.changed).toBe(0);
    expect(view.lines.map(line => line.text).join("\n")).toContain("\"message\": \"Use the logger\"");
    expect(view.body).toEqual({ owner: "library", id: "mine", contentHash: "h1" });
  });

  it("shows an edit against the approved content, which runs meanwhile", () => {
    const view = approvalView(pending({
      rule: codeRule("mine", { message: "edited words" }),
      approved: { rule: codeRule("mine", { message: "approved words" }), approvedAt: 1_000 },
    }));
    expect(view.runs).toBe("approved");
    expect(view.approvedAt).toBe(1_000);
    expect(view.lines.filter(line => line.op === "removed").map(line => line.text)).toEqual([expect.stringContaining("approved words")]);
    expect(view.lines.filter(line => line.op === "added").map(line => line.text)).toEqual([expect.stringContaining("edited words")]);
  });

  it("shows invisible characters in the content and the title as escapes", () => {
    const view = approvalView(pending({ rule: codeRule("mine", { message: "Use the logger\u200b\u202e" }) }));
    expect(view.title).toBe("Use the logger\\u200b\\u202e");
    expect(view.lines.some(line => line.text.includes("Use the logger\\u200b\\u202e"))).toBe(true);
    expect(view.lines.some(line => /[\p{Cf}\p{Co}\p{Cn}]/u.test(line.text))).toBe(false);
  });

  it("names its owner: created by you, or the rule pack it comes in", () => {
    expect(ownerText(pending())).toBe("Created by you");
    expect(ownerText(pending({ owner: "pack", packId: "house" }), id => (id === "house" ? "House rules" : id))).toContain("House rules");
  });

  it("counts every rule waiting, and none before the list is read", () => {
    expect(awaitingCount(null)).toBe(0);
    expect(awaitingCount([pending(), pending({ id: "other" })])).toBe(2);
  });
});

describe("what a rule row says of its approval", () => {
  it("says nothing of content that runs as approved or as shipped", () => {
    expect(pendingNote({ state: "approved", approvedAt: 1 })).toBeNull();
    expect(pendingNote({ state: "shipped", approvedAt: null })).toBeNull();
    expect(pendingNote(undefined)).toBeNull();
  });

  it("tells an edit whose approved content runs meanwhile from a rule that runs nowhere", () => {
    expect(pendingNote({ state: "pending", approvedAt: 1 })?.runs).toBe("approved");
    expect(pendingNote({ state: "pending", approvedAt: null })?.runs).toBe("nowhere");
  });
});

describe("the rules a repository holds that wait for approval", () => {
  const unavailable = [
    { sourceKind: "library_rule", sourceId: "mine", folder: "", reason: "awaiting-approval" },
    { sourceKind: "library_rule", sourceId: "mine", folder: "web", reason: "awaiting-approval" },
    { sourceKind: "rerail_rule", sourceId: "house:a", folder: "", reason: "awaiting-approval" },
    { sourceKind: "pack", sourceId: "gone", folder: "", reason: "missing" },
  ];

  it("lists each waiting rule once with every folder that adds it, and not a missing one", () => {
    expect(awaitingReferences(unavailable)).toEqual([
      { sourceKind: "library_rule", id: "mine", folders: ["", "web"] },
      { sourceKind: "rerail_rule", id: "house:a", folders: [""] },
    ]);
    expect(awaitingReferences(undefined)).toEqual([]);
  });

  it("shows a rule created by you on Created by you and a pack's rule on its pack's page", () => {
    const references = awaitingReferences(unavailable);
    const approvals = [pending({ owner: "pack", id: "house:a", packId: "house" })];
    expect(awaitingOnPage(references, "mine", approvals).map(r => r.id)).toEqual(["mine"]);
    expect(awaitingOnPage(references, "pack:house", approvals).map(r => r.id)).toEqual(["house:a"]);
    expect(awaitingOnPage(references, "pack:other", approvals)).toEqual([]);
    // Before the waiting list is read, the pack the catalog names is enough.
    expect(awaitingOnPage(references, "pack:house", null, id => (id === "house:a" ? "house" : undefined)).map(r => r.id)).toEqual(["house:a"]);
  });

  it("titles a waiting rule from its waiting content, else from what the pack calls it", () => {
    const [mine, house] = awaitingReferences(unavailable);
    expect(waitingTitle(mine!, [pending()])).toBe("Use the logger");
    expect(waitingTitle(house!, null, () => "No console logs")).toBe("No console logs");
    expect(waitingTitle(house!, null)).toBe("house:a");
  });

  it("shows Waiting for approval without a repository, as Settings", () => {
    setScope(null);
    ui.view = "approvals";
    expect(pageKey()).toBe("approvals");
    ui.view = "history";
    expect(pageKey()).toBe("no-repo");
  });
});

describe("approving rules from the dashboard, against the API", () => {
  let root: string;
  let databasePath: string;
  let stores: DatabaseApiStores;

  beforeEach(async () => {
    root = realpathSync(await mkdtemp(join(tmpdir(), "rerail-ui-approvals-")));
    const checkout = join(root, "app");
    mkdirSync(checkout, { recursive: true });
    execFileSync("git", ["init", "-q", "-b", "main"], { cwd: checkout, stdio: "ignore" });
    databasePath = join(root, "rerail.sqlite");
    stores = new DatabaseApiStores(databasePath, {
      health: () => ({ pid: process.pid, database: { path: databasePath, exists: true }, native: { ok: true } }),
      origin: () => "http://127.0.0.1:4737",
    });
    const db = stores.database(), catalog = stores.ruleCatalog();
    const repositoryKey = recordCheckout(db, checkout).repositoryKey;
    setUpRepository(db, repositoryKey, { packs: [] });
    // A coding agent's proposals: a new rule, and an edit of a rule a person approved.
    upsertLibraryRule(db, catalog, codeRule("ui-new", { message: "Use the logger\u200b, not print" }), "code");
    upsertLibraryRule(db, catalog, codeRule("ui-edited", { message: "approved words" }), "code", { approve: true });
    upsertLibraryRule(db, catalog, codeRule("ui-edited", { message: "edited words" }), "code");
    for (const id of ["ui-new", "ui-edited"]) addAttachment(db, { repositoryKey, folder: "", sourceKind: "library_rule", sourceId: id });

    const app = createRerailApi(stores, operations, { version: "0.0.0-test" });
    vi.stubGlobal("location", { origin: "http://127.0.0.1:4737" });
    // The daemon serves /healthz beside the API.
    vi.stubGlobal("fetch", async (input: URL, init?: RequestInit) => {
      const url = new URL(input);
      return url.pathname === "/healthz" ? Response.json({ version: "0.0.0-test", pid: process.pid }) : app.request(url.href, init);
    });
    setScope({ repository: repositoryKey });
    ui.view = "approvals";
    setNeeds(currentNeeds);
  });

  afterEach(async () => {
    vi.unstubAllGlobals();
    Object.assign(data, { approvals: null, awaiting: [], awaitingApproval: 0, rules: new Map(), repository: null, checkout: null });
    stores.close();
    await rm(root, { recursive: true, force: true });
  });

  const approvals = () => data.approvals as PendingRule[] | null;
  const viewOf = (id: string) => approvalView(approvals()!.find(p => p.id === id)!);

  it("shows how many rules wait, each one's content, and the count drops as they are approved", async () => {
    await refresh();

    expect(pageKey()).toBe("approvals");
    expect(awaitingCount(approvals())).toBe(2);
    expect(data.awaitingApproval).toBe(2);
    expect(viewOf("ui-new")).toMatchObject({ runs: "nowhere", owner: "Created by you" });
    expect(viewOf("ui-new").lines.some(line => line.text.includes("Use the logger\\u200b, not print"))).toBe(true);
    expect(viewOf("ui-edited").runs).toBe("approved");
    expect(viewOf("ui-edited").lines.filter(line => line.op !== "same").map(line => line.text)).toEqual([
      expect.stringContaining("approved words"), expect.stringContaining("edited words"),
    ]);
    // The repository's rules: the edited one runs its approved content; the new one is not missing, it waits.
    expect(pendingNote((data.rules.get("ui-edited") as StoredRule).approval)?.runs).toBe("approved");
    expect(data.rules.has("ui-new")).toBe(false);
    expect(awaitingOnPage(data.awaiting, "mine", approvals()).map(r => r.id)).toEqual(["ui-new"]);

    await send("POST", "/api/rule-approvals", viewOf("ui-edited").body);
    await refresh();
    expect(awaitingCount(approvals())).toBe(1);
    expect(pendingNote((data.rules.get("ui-edited") as StoredRule).approval)).toBeNull();

    await send("POST", "/api/rule-approvals", viewOf("ui-new").body);
    await refresh();
    expect(awaitingCount(approvals())).toBe(0);
    expect(data.awaiting).toEqual([]);
    expect(data.rules.has("ui-new")).toBe(true);
  });

  it("refuses content that changed after the page read it, and the page reads the new content", async () => {
    await refresh();
    const read = viewOf("ui-new").body;
    const elsewhere = openRerailDatabase(databasePath);
    try {
      upsertLibraryRule(elsewhere, stores.ruleCatalog(), codeRule("ui-new", { message: "swapped in after the person read it" }), "code");
    } finally {
      elsewhere.$client.close();
    }

    const refused = await send("POST", "/api/rule-approvals", read).then(() => null, (error: unknown) => error);
    expect(isRuleChanged(refused)).toBe(true);

    await refresh();
    expect(awaitingCount(approvals())).toBe(2);
    expect(viewOf("ui-new").body.contentHash).not.toBe(read.contentHash);
    expect(viewOf("ui-new").lines.some(line => line.text.includes("swapped in after the person read it"))).toBe(true);
  });

  it("does not take another refusal for a changed rule", () => {
    expect(isRuleChanged(new Error("POST /api/rule-approvals answered 409: {\"error\":\"approval-not-needed\"}"))).toBe(false);
    expect(isRuleChanged(new Error("POST /api/rule-approvals answered 404: {\"error\":\"rule-not-found\"}"))).toBe(false);
  });
});
