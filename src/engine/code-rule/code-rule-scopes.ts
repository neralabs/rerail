/**
 * Which files the code rules of one catalog version read, for a checkout no lane is open in. A lane's native workspace
 * holds its own copy of every code rule with that checkout's placement, plus the checkout's Git snapshot for the content
 * store: about 6 MB outside the JavaScript heap. A session's walk asks which files each worktree of its repository
 * reads, so a lane per checkout asked about held gigabytes where a few checkouts are edited. A `CodeRuleScopes` holds
 * the code rules and their placements alone (no root, no store) and answers as a lane on that catalog version does
 * (`workspaceReadsPath`). Every checkout and session whose catalog has that version shares one, and the last holder to
 * let go closes it.
 */
import { loadNativeBinding, type NativeCodeRuleDslWorkspace } from "../native/native.js";
import type { CompiledCodeRuleDslSnapshot } from "./code-rule-dsl-catalog.js";
import { workspaceReadsPath } from "./code-rule-enforcer.js";

export class CodeRuleScopes {
  /** The scopes of each catalog version some holder holds. */
  static readonly #held = new Map<string, CodeRuleScopes>();
  readonly version: string;
  readonly #workspace: NativeCodeRuleDslWorkspace | undefined;
  readonly #holders = new Set<object>();

  private constructor(snapshot: CompiledCodeRuleDslSnapshot) {
    this.version = snapshot.version;
    this.#workspace = scopeWorkspace(snapshot);
  }

  /** The scopes of `snapshot`'s catalog version, made on the first hold, now held by `holder` too until it lets go. */
  static hold(snapshot: CompiledCodeRuleDslSnapshot, holder: object): CodeRuleScopes {
    let scopes = CodeRuleScopes.#held.get(snapshot.version);
    if (scopes === undefined) {
      scopes = new CodeRuleScopes(snapshot);
      CodeRuleScopes.#held.set(snapshot.version, scopes);
    }
    scopes.#holders.add(holder);
    return scopes;
  }

  /** Whether the code rules read `path`, relative to a checkout of this catalog version, apart from Git's ignore rules. */
  readsPath(path: string): boolean {
    return workspaceReadsPath(this.#workspace, path);
  }

  /** `holder` lets go; the last holder's release closes the native workspace. */
  release(holder: object): void {
    if (!this.#holders.delete(holder) || this.#holders.size > 0) return;
    if (CodeRuleScopes.#held.get(this.version) === this) CodeRuleScopes.#held.delete(this.version);
    this.#workspace?.close();
  }
}

/**
 * A workspace holding `snapshot`'s code rules, each with its placement, as a lane's `#prepareWorkspace` adds them;
 * undefined where the engine refuses one, which leaves the answers those of a lane whose workspace did not open.
 */
function scopeWorkspace(snapshot: CompiledCodeRuleDslSnapshot): NativeCodeRuleDslWorkspace | undefined {
  const workspace = new (loadNativeBinding().CodeRuleDslWorkspace)();
  try {
    for (const codeRule of snapshot.codeRules) workspace.addCodeRule(codeRule.executable, JSON.stringify(codeRule.placement));
    return workspace;
  } catch (error) {
    workspace.close();
    // The lane an audit opens on this catalog records the failure; until then this answers as that lane does.
    process.stderr.write(`rerail: cannot read the code rules of catalog ${snapshot.version}: ${error instanceof Error ? error.message : String(error)}\n`);
    return undefined;
  }
}
