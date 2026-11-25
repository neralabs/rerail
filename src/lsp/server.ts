/**
 * Internal testing: `rerail lsp`, a Language Server over stdio that judges each open document's text, saved or not,
 * by the code rules of the checkout that owns it, and publishes their Violations as diagnostics. It runs the rules in
 * its own process, as a headless scan does: a log-mode `touched-files` audit per document version, recorded in memory
 * and never in the Rerail database, so it steers no agent and adds nothing to any session.
 *
 * A `.rerail` rule file is read by the engine's own parser too (`rerail-file.ts`): its tokens served as semantic
 * tokens, for highlighting, and its syntax errors and the engine's check of it published with the rules' diagnostics.
 */
import { randomUUID } from "node:crypto";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { Readable, Writable } from "node:stream";
import { fileURLToPath } from "node:url";
import {
  CodeActionKind,
  TextDocumentSyncKind,
  TextDocuments,
  createConnection,
  type CodeAction,
  type Connection,
  type Diagnostic,
} from "vscode-languageserver/node";
import { TextDocument } from "vscode-languageserver-textdocument";
import type { AuditInput, AuditRecorder, AuditResult } from "../engine/audit/audit.js";
import { CheckoutOwners } from "../engine/catalog/checkout-owner.js";
import { CheckoutCodeRuleRouter } from "../engine/code-rule/checkout-code-rule-router.js";
import { CheckoutCatalogs } from "../engine/code-rule/code-rule-dsl-catalog.js";
import { languageFor } from "../engine/code-rule/code-rule-enforcer.js";
import { UNLISTED_TEXT_LANGUAGE } from "../engine/code-rule/code-rule-languages.js";
import { steerDashboard } from "../presentation/session-presentation.js";
import { JudgedText, diagnosticFix, violationDiagnostic } from "./diagnostics.js";
import { RERAIL_TOKEN_LEGEND, checkRerailRule, isRerailDocument, readRerailText, type RerailText } from "./rerail-file.js";

export interface LanguageServerOptions {
  /** Reported to the editor as the server's version. */
  readonly version: string;
  readonly input?: Readable;
  readonly output?: Writable;
}

/** How long a document stays unchanged before its text is judged. */
const CHANGE_DELAY_MS = 300;

/** The text sent for one audit of a document, kept to map the audit's byte spans onto it. */
interface Judged {
  readonly revision: number;
  readonly document: TextDocument;
}

/** Where a document's diagnostics come from: each source's latest replaces only its own. */
type DiagnosticOrigin = "rules" | "syntax" | "check";

/** Each document's diagnostics by origin, published together: a publish replaces all of a document's diagnostics. */
class PublishedDiagnostics {
  readonly #connection: Connection;
  readonly #documents = new Map<string, Map<DiagnosticOrigin, Diagnostic[]>>();

  constructor(connection: Connection) {
    this.#connection = connection;
  }

  set(uri: string, version: number | undefined, origin: DiagnosticOrigin, diagnostics: Diagnostic[]): void {
    const origins = this.#documents.get(uri) ?? new Map<DiagnosticOrigin, Diagnostic[]>();
    origins.set(origin, diagnostics);
    this.#documents.set(uri, origins);
    const all = [...origins.values()].flat();
    void this.#connection.sendDiagnostics({ uri, ...(version === undefined ? {} : { version }), diagnostics: all });
  }

  forget(uri: string): void {
    this.#documents.delete(uri);
    void this.#connection.sendDiagnostics({ uri, diagnostics: [] });
  }
}

/** Each document's latest audit is published; one a later revision superseded is dropped. */
class DocumentLinter implements AuditRecorder {
  readonly #connection: Connection;
  readonly #published: PublishedDiagnostics;
  readonly #sessionId = `lsp-${randomUUID()}`;
  readonly #catalogs = new CheckoutCatalogs();
  readonly #owners = new CheckoutOwners(() => this.#catalogs.recordedCheckoutsVersion());
  readonly #routers = new Map<string, Promise<CheckoutCodeRuleRouter>>();
  readonly #dashboards = new Map<string, string>();
  readonly #audits = new Map<string, AuditInput>();
  /** Absolute path → the text its latest audit judges. */
  readonly #latest = new Map<string, Judged>();
  #revision = 0;
  #nextAuditId = 0;

  constructor(connection: Connection, published: PublishedDiagnostics) {
    this.#connection = connection;
    this.#published = published;
  }

  async check(document: TextDocument): Promise<void> {
    if (!document.uri.startsWith("file:")) return;
    const absolute = fileURLToPath(document.uri);
    const owner = this.#owners.ownerOf(absolute);
    if (owner === undefined) return;
    const router = await this.#router(owner.checkoutRoot);
    router.refreshCodeRules();
    const path = owner.path;
    if (!router.acceptsPath(path) || router.ignoresPath(path, false)) {
      this.#latest.delete(join(owner.checkoutRoot, path));
      this.#publish(document.uri, document.version, []);
      return;
    }
    const revision = ++this.#revision;
    const text = document.getText();
    this.#latest.set(join(owner.checkoutRoot, path), {
      revision,
      document: TextDocument.create(document.uri, document.languageId, document.version, text),
    });
    void this.#dashboard(owner.checkoutRoot);
    await router.auditGeneratedEdits(this.#sessionId, {
      revision,
      checkpoints: ["edit"],
      mode: "log",
      edits: [{ path, language: languageFor(path) ?? UNLISTED_TEXT_LANGUAGE, generatedText: text }],
    });
  }

  forget(document: TextDocument): void {
    if (document.uri.startsWith("file:")) {
      const owner = this.#owners.ownerOf(fileURLToPath(document.uri));
      if (owner !== undefined) this.#latest.delete(join(owner.checkoutRoot, owner.path));
    }
  }

  async close(): Promise<void> {
    const routers = await Promise.allSettled(this.#routers.values());
    this.#routers.clear();
    await Promise.allSettled(routers.flatMap(router => (router.status === "fulfilled" ? [router.value.close()] : [])));
    this.#catalogs.close();
  }

  start(input: AuditInput): string {
    const auditId = String(this.#nextAuditId++);
    this.#audits.set(auditId, input);
    return auditId;
  }

  finish(auditId: string, result: AuditResult): void {
    const input = this.#audits.get(auditId);
    this.#audits.delete(auditId);
    if (input === undefined) return;
    for (const path of input.paths) {
      const judged = this.#latest.get(join(input.workspaceRoot, path));
      if (judged?.revision !== input.revision) continue;
      for (const failure of result.failures ?? []) this.#connection.console.warn(`${path}: ${failure.code} ${failure.message}`);
      if (result.status !== "completed") continue;
      const text = new JudgedText(judged.document);
      const dashboard = this.#dashboards.get(input.workspaceRoot);
      const diagnostics = (result.violations ?? [])
        .filter(violation => violation.path === path)
        .map(violation => violationDiagnostic(violation, text, dashboard));
      this.#publish(judged.document.uri, judged.document.version, diagnostics);
    }
  }

  progress(): void {}

  interrupt(): void {}

  #publish(uri: string, version: number | undefined, diagnostics: Diagnostic[]): void {
    this.#published.set(uri, version, "rules", diagnostics);
  }

  #router(root: string): Promise<CheckoutCodeRuleRouter> {
    let router = this.#routers.get(root);
    if (router === undefined) {
      const created = new CheckoutCodeRuleRouter({
        sessionRoot: root,
        startingCheckout: root,
        owners: this.#owners,
        catalogs: this.#catalogs,
        steerDelivery: { requestSteer: async () => undefined },
        audits: this,
        scope: "touched-files",
        mode: "log",
        followGitignore: this.#catalogs.followsGitignore(root),
      });
      router = created.openWorkspace(this.#sessionId).then(() => created);
      this.#routers.set(root, router);
    }
    return router;
  }

  /** The `why?` links' dashboard for `root`, once a daemon answers; asked again while none has. */
  async #dashboard(root: string): Promise<void> {
    if (this.#dashboards.has(root)) return;
    const dashboard = await steerDashboard(root);
    if (dashboard !== undefined) this.#dashboards.set(root, dashboard);
  }
}

/** Serves the Language Server Protocol on `input` and `output` (stdio by default) until the editor exits. */
export function runRerailLanguageServer(options: LanguageServerOptions): void {
  // The machine-wide content store belongs to the daemon; this process keeps its own, removed when it exits.
  if ((process.env.RERAIL_CONTENT_STORE_PATH ?? "") === "") {
    const scratch = mkdtempSync(join(tmpdir(), "rerail-lsp-"));
    process.env.RERAIL_CONTENT_STORE_PATH = join(scratch, "content.sqlite");
    process.on("exit", () => rmSync(scratch, { recursive: true, force: true }));
  }
  const connection = createConnection(options.input ?? process.stdin, options.output ?? process.stdout);
  const documents = new TextDocuments(TextDocument);
  const published = new PublishedDiagnostics(connection);
  const linter = new DocumentLinter(connection, published);
  const timers = new Map<string, NodeJS.Timeout>();
  /** Each open `.rerail` document's latest read, kept for its version: its tokens and its syntax errors. */
  const rerailReads = new Map<string, { readonly version: number; readonly read: RerailText }>();
  /** Each open `.rerail` document's version the engine last checked as saved. */
  const rerailChecked = new Map<string, number>();
  const failed = (uri: string) => (error: unknown): void => {
    connection.console.error(`${uri}: ${error instanceof Error ? error.message : String(error)}`);
  };

  const rerailRead = (document: TextDocument): RerailText => {
    const kept = rerailReads.get(document.uri);
    if (kept?.version === document.version) return kept.read;
    const read = readRerailText(document);
    rerailReads.set(document.uri, { version: document.version, read });
    return read;
  };

  /** A `.rerail` document's syntax errors; and, `saved`, the engine's check of the file, kept until an edit. */
  const readRerail = (document: TextDocument, saved: boolean): void => {
    if (!isRerailDocument(document)) return;
    const { uri, version } = document;
    try {
      published.set(uri, version, "syntax", rerailRead(document).errors);
    } catch (error) {
      failed(uri)(error);
    }
    if (!saved) {
      if (rerailChecked.get(uri) !== version) published.set(uri, version, "check", []);
      return;
    }
    rerailChecked.set(uri, version);
    checkRerailRule(document).then(diagnostics => {
      if (documents.get(uri)?.version === version) published.set(uri, version, "check", diagnostics);
    }, failed(uri));
  };

  const check = (document: TextDocument): void => {
    clearTimeout(timers.get(document.uri));
    timers.delete(document.uri);
    linter.check(document).catch(failed(document.uri));
  };

  connection.onInitialize(() => ({
    capabilities: {
      textDocumentSync: { openClose: true, change: TextDocumentSyncKind.Incremental, save: { includeText: false } },
      codeActionProvider: { codeActionKinds: [CodeActionKind.QuickFix] },
      semanticTokensProvider: { legend: RERAIL_TOKEN_LEGEND, full: true },
    },
    serverInfo: { name: "rerail", version: options.version },
  }));
  documents.onDidOpen(({ document }) => {
    readRerail(document, true);
    check(document);
  });
  documents.onDidSave(({ document }) => {
    readRerail(document, true);
    check(document);
  });
  documents.onDidChangeContent(({ document }) => {
    clearTimeout(timers.get(document.uri));
    timers.set(document.uri, setTimeout(() => {
      readRerail(document, false);
      check(document);
    }, CHANGE_DELAY_MS));
  });
  documents.onDidClose(({ document }) => {
    clearTimeout(timers.get(document.uri));
    timers.delete(document.uri);
    rerailReads.delete(document.uri);
    rerailChecked.delete(document.uri);
    linter.forget(document);
    published.forget(document.uri);
  });
  connection.languages.semanticTokens.on(({ textDocument }) => {
    const document = documents.get(textDocument.uri);
    if (document === undefined || !isRerailDocument(document)) return { data: [] };
    return { data: rerailRead(document).tokens };
  });
  connection.onCodeAction(({ textDocument, context }) =>
    context.diagnostics.flatMap((diagnostic): CodeAction[] => {
      const fix = diagnosticFix(diagnostic);
      if (fix === undefined) return [];
      return [{
        title: `Apply Rerail's suggestion (${String(diagnostic.code)})`,
        kind: CodeActionKind.QuickFix,
        diagnostics: [diagnostic],
        edit: { changes: { [textDocument.uri]: [{ range: fix.range, newText: fix.text }] } },
      }];
    }));
  connection.onShutdown(async () => {
    for (const timer of timers.values()) clearTimeout(timer);
    timers.clear();
    await linter.close();
  });
  documents.listen(connection);
  connection.listen();
}
