/**
 * The repository-owned rule model: the types every module agrees on.
 *
 * A repository owns its rule configuration. It is recognised on this machine by its key and has checkouts
 * a repository, optionally in one folder. Rule settings change a rule's or a pack's `on` / `mode` / `suppressions`,
 * or a rule's option values (`config`), and belong to the repository or to one checkout. Precedence per field
 * (per option for `config`): this checkout, its parent checkouts (nearest first), the repository; within one owner
 * the deepest folder; within one folder the rule before its pack; then the pack's starting value, then the rule's
 * own default.
 */
import type { CodeRuleDslValue } from "../code-rule/code-rule.js";
import type { RuleMode } from "./rule-mode.js";
import type { RulePack } from "./rule-packs.js";
import type { RuleDocument } from "./rule-store.js";

/** `remote:<host>/<path>` from the normalised `origin` (or sole) remote address, else `clone:<clone id>`. */
export type RepositoryKey = string;

export interface RepositoryRecord {
  readonly repositoryKey: RepositoryKey;
  readonly displayName: string;
  /** Other keys that resolve to this repository (renames, ssh host aliases, mirrors). */
  readonly aliases: readonly RepositoryKey[];
  /** When set-up ran; `null` means Not set up, and nothing runs. */
  readonly configuredAt: number | null;
  readonly started: boolean;
  /** Whether audits leave out files Git ignores; true until turned off. */
  readonly followGitignore: boolean;
  readonly createdAt: number;
  readonly updatedAt: number;
}

export interface CheckoutRecord {
  /** Physical (symlink-resolved) git top-level of the checkout. */
  readonly checkoutPath: string;
  readonly repositoryKey: RepositoryKey;
  /** The id git already gives the clone (`rerail-repository-id` in the git common dir). */
  readonly cloneId: string;
  /** `null` follows the parent checkout, then the repository. */
  readonly started: boolean | null;
  readonly createdAt: number;
  readonly lastSeenAt: number;
}

/** A recorded (or passively looked-up) checkout. */
export interface CheckoutContext {
  readonly checkoutPath: string;
  readonly repositoryKey: RepositoryKey;
  /** `null`: the clone has no id file yet (passive lookup only); `recordCheckout` always returns a string. */
  readonly cloneId: string | null;
  /** False for a folder outside git (identity in `<folder>/.rerail/repository-id`). */
  readonly git: boolean;
  /** True for a clone's main checkout, false for a linked worktree. */
  readonly main: boolean;
}

/** `not-set-up`: started, but the repository has no configuration, so nothing runs. */
export type RepositoryStartState = "started" | "stopped" | "not-set-up";

export interface RepositoryStatus {
  readonly started: boolean;
  readonly setUp: boolean;
  readonly startedBy: "checkout" | "parent" | "repository";
}

export const RULE_SOURCE_KINDS = ["pack", "rerail_rule", "library_rule"] as const;
export type RuleSourceKind = (typeof RULE_SOURCE_KINDS)[number];

export const RULE_SETTING_OWNER_KINDS = ["repository", "checkout"] as const;
export type RuleSettingOwnerKind = (typeof RULE_SETTING_OWNER_KINDS)[number];

export const RULE_SETTING_TARGET_KINDS = ["rule", "pack"] as const;
export type RuleSettingTargetKind = (typeof RULE_SETTING_TARGET_KINDS)[number];

/** Whether a linter's suppression comment (`// eslint-disable-line`, `# noqa`, ...) silences a ported rule. */
export const RULE_SUPPRESSIONS = ["honour", "ignore"] as const;
export type RuleSuppressions = (typeof RULE_SUPPRESSIONS)[number];

/** Option values for a code rule by Rerail, by option name: they replace the values its `define.options` declare. */
export type RuleConfig = Readonly<Record<string, CodeRuleDslValue>>;

export type SettingOwner =
  | { readonly kind: "repository"; readonly repositoryKey: RepositoryKey }
  | { readonly kind: "checkout"; readonly checkoutPath: string };

/** Adds a pack, a By Rerail rule or a library rule to a repository, in `folder` (`""` = whole repository). */
export interface RuleAttachment {
  readonly repositoryKey: RepositoryKey;
  readonly folder: string;
  readonly sourceKind: RuleSourceKind;
  readonly sourceId: string;
}

/**
 * Changes `on`, `mode` and/or `suppressions` of a rule or a whole pack, or the option values (`config`) of one rule
 * by Rerail; `null` (or omitted) leaves the field to the next owner.
 */
export interface RuleSetting {
  readonly owner: SettingOwner;
  readonly folder: string;
  readonly targetKind: RuleSettingTargetKind;
  readonly targetId: string;
  readonly on: boolean | null;
  readonly mode: RuleMode | null;
  readonly suppressions?: RuleSuppressions | null;
  readonly config?: RuleConfig | null;
}

/** What decided one field of an effective rule. */
export type Decider =
  | { readonly by: "checkout"; readonly checkoutPath: string; readonly folder: string; readonly target: RuleSettingTargetKind }
  | { readonly by: "repository"; readonly folder: string; readonly target: RuleSettingTargetKind }
  | { readonly by: "pack-start"; readonly packId: string }
  | { readonly by: "rule-default" };

export interface EffectiveRule {
  readonly document: RuleDocument;
  /** Where the rule comes from. A rule in an attached pack belongs to that pack even if also attached alone. */
  readonly source: { readonly kind: RuleSourceKind; readonly id: string; readonly packId: string | null };
  /** Folder of the attachment that made the rule present. */
  readonly attachmentFolder: string;
  readonly on: boolean;
  readonly mode: RuleMode;
  /** `ignore`: no linter suppression comment silences the rule. Defaults from the pack's `honourSuppressionComments`. */
  readonly suppressions: RuleSuppressions;
  /**
   * Option values set by rule settings, merged per option; `null` when no setting sets one. Applies to code rules by
   * Rerail, wherever the rule runs in the checkout: only a setting for the whole repository (folder `""`) sets it.
   */
  readonly config: RuleConfig | null;
  /** Why `config` could not be applied to the document (an option it no longer declares, ...); the rule runs nowhere. */
  readonly configError: string | null;
  readonly decidedBy: {
    readonly on: Decider;
    readonly mode: Decider;
    readonly suppressions: Decider;
    /** The setting that decided each option in `config`. */
    readonly config: Readonly<Record<string, Decider>>;
  };
  readonly revision: number;
  readonly payloadHash: string;
  /** Whose content runs: Nera's, or the content a person approved (`rule-approval.ts`). */
  readonly approval: RuleApprovalStatus;
}

/**
 * How a rule's content stands with approval. `shipped`: Nera shipped it, so it needs none (`approvedAt` null).
 * `approved`: a person approved this content. `pending`: newer content waits for approval; the content approved at
 * `approvedAt` runs meanwhile, or, with `approvedAt` null, the rule runs nowhere yet.
 */
export interface RuleApprovalStatus {
  readonly state: "shipped" | "approved" | "pending";
  readonly approvedAt: number | null;
}

/**
 * An attachment whose target can't be resolved, or whose rule has no approved content yet
 * (`awaiting-approval`); reported, never thrown.
 */
export interface UnavailableReference {
  readonly sourceKind: RuleSourceKind;
  readonly sourceId: string;
  readonly folder: string;
  readonly reason: "missing" | "conflicts-with-library" | "awaiting-approval";
}

/** The effective configuration for one checkout. Empty unless the checkout is started and its repository set up. */
export interface CheckoutRuleSet {
  readonly checkout: CheckoutContext;
  readonly status: RepositoryStatus;
  /** Changes whenever anything that could change `rulesFor` / `conversationRules` changes. */
  readonly signature: string;
  /** Code rules for one path relative to the checkout root. */
  rulesFor(relativePath: string): readonly EffectiveRule[];
  /** Conversation rules for the session's starting folder (relative to the checkout root). */
  conversationRules(startFolder: string): readonly EffectiveRule[];
  readonly unavailable: readonly UnavailableReference[];
}

export interface LibraryRule {
  readonly document: RuleDocument;
  readonly revision: number;
  readonly payloadHash: string;
  /** The content digest of the stored body (`ruleContentDigest`), which an approval binds to. */
  readonly contentHash: string;
  /** Set when the rule is an edited copy of a By Rerail rule. */
  readonly copiedFromRuleId: string | null;
  readonly createdAt: number;
  readonly updatedAt: number;
}

/** An installed pack the catalog left out, and why. */
export interface SkippedRulePack {
  /** The pack's directory. */
  readonly path: string;
  /** The npm package and version its `package.json` names, where it can be read. */
  readonly packageName: string | null;
  readonly version: string | null;
  readonly reason: string;
}

/** The rules and packs installed on this machine (`rule-catalog.ts`). */
export interface RuleCatalog {
  readonly documents: ReadonlyMap<string, RuleDocument>;
  readonly packs: ReadonlyMap<string, RulePack>;
  /** Rule id → id of the pack that holds it (a rule is in at most one pack). */
  readonly packOf: ReadonlyMap<string, string>;
  /**
   * Rule id → the code rule's document as written, its `$option` references unresolved (`documents` holds it
   * resolved). A rule setting's `config` is applied to this one.
   */
  readonly sources: ReadonlyMap<string, unknown>;
  /**
   * Ids of the rules Nera shipped: those of packs installed from the npm registry. They run as shipped; any other
   * rule runs only with content a person approved (`rule-approval.ts`).
   */
  readonly shipped: ReadonlySet<string>;
  /** Changes whenever a pack is added, removed or updated, or comes from another kind of install. */
  readonly digest: string;
  /** Installed packs left out because they failed a check; the others load. */
  readonly skipped: readonly SkippedRulePack[];
}

/**
 * Native placement of one compiled code rule in one checkout: for a path, the entry with the deepest folder that
 * contains it decides; `mode: null` means off there. A path no entry contains is not evaluated. `suppressions:
 * "ignore"` means no linter suppression comment silences a violation there; omitted means honour. Folders are
 * normalised (see {@link normalizeFolder}); entries are sorted by folder.
 */
export interface CodeRulePlacementEntry {
  readonly folder: string;
  readonly mode: RuleMode | null;
  readonly suppressions?: "ignore";
}
export type CodeRulePlacement = readonly CodeRulePlacementEntry[];

/**
 * Who is calling the API: `/api` callers are people, `/mcp` callers are coding agents. An agent's checkout scopes the
 * history its tools return to its own repository and keeps the person's choices (Follow .gitignore off, set-up
 * repositories' rules) out of its reach; it is not authentication, since any local process can call `/api`.
 */
export type ApiCaller =
  | { readonly kind: "person" }
  | { readonly kind: "agent"; readonly checkout: string | undefined; readonly folder: string };

/** Header the MCP bridge sends on every request: the URI-encoded real path of the folder it was started in. */
export const CALLER_CWD_HEADER = "x-rerail-caller-cwd";

/**
 * A folder relative to the repository root, in POSIX form: `""` is the whole repository, otherwise no leading or
 * trailing `/`, no `.` or `..` segments. Throws on an absolute path or `..`.
 */
export function normalizeFolder(folder: string): string {
  const posix = folder.replaceAll("\\", "/").trim();
  if (posix.startsWith("/") || /^[A-Za-z]:\//.test(posix)) throw new Error(`Folder must be relative to the repository: ${folder}`);
  const segments = posix.split("/").filter(segment => segment !== "" && segment !== ".");
  if (segments.includes("..")) throw new Error(`Folder must stay inside the repository: ${folder}`);
  return segments.join("/");
}

/** Whether `folder` (normalised) contains `path` (relative to the repository root, POSIX). `web` does not contain `web2/x`. */
export function folderContains(folder: string, path: string): boolean {
  return folder === "" || path === folder || path.startsWith(`${folder}/`);
}
