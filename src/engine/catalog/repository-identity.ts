import { randomUUID } from "node:crypto";
import {
  existsSync,
  chmodSync,
  closeSync,
  constants as fsConstants,
  fstatSync,
  linkSync,
  lstatSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  readSync,
  unlinkSync,
  writeFileSync,
} from "node:fs";
import { basename, dirname, join, resolve } from "node:path";
import { checkoutRealpath, physicalPath, storedForm, storedPath } from "./checkout-control.js";
import {
  filePlacementInputs,
  fileText,
  forgetGoneConfigs,
  gitEntryFacts,
  linkedWorktreeFromFiles,
  lookupEnvironment,
  placementInputs,
  realOf,
  searchedIn,
  searchStep,
  type PlacementInputs,
} from "./checkouts/git-placement.js";
import { foldersGitSearches } from "./checkouts/git-search.js";
import { openNoFollowSync } from "../no-follow.js";
import { GitUnavailableError, gitHome, gitText, runGit, runGitAsync, type GitResult } from "../git.js";
import type { RepositoryKey } from "./repository-model.js";

/** Where a checkout keeps its clone id, and which checkout it is. */
export interface RepositoryIdentityLocation {
  git: boolean;
  /** The checkout's top folder: the main checkout's, or a linked worktree's own. */
  workspaceRoot: string;
  identityPath: string;
  /** False only for a linked Git worktree; a folder outside Git is its own main checkout. */
  main: boolean;
}

function nonGitLocation(requested: string): RepositoryIdentityLocation {
  return { git: false, workspaceRoot: requested, identityPath: join(requested, ".rerail", "repository-id"), main: true };
}

/** The file in Git's common metadata directory that holds a clone's id, shared by every linked worktree. */
export const GIT_IDENTITY_FILE = "rerail-repository-id";

/** What {@link gitLocation} reads: Git's top level, common directory and Git directory, as absolute paths. */
const LOCATE_ARGS = ["rev-parse", "--show-toplevel", "--path-format=absolute", "--git-common-dir", "--git-dir"];

/**
 * Git's answers for the folders it took for their own checkout's top level, each kept while what Git read to give it,
 * the real paths it printed and whether Git takes the folders for the user's are unchanged
 * ({@link PlacementInputs.stamp}): a daemon asks after every registered worktree each time a repository starts, in every
 * session's runtime, and once a checkout is recorded every runtime asks again. Each holds the answer's common Git folder
 * in its {@link storedForm} too, read with the placement inputs ({@link forgetGone}).
 */
const heldTopLevels = new Map<string, { readonly stamp: string; readonly location: RepositoryIdentityLocation; readonly common: string }>();

/**
 * The common Git folders of the checkouts Git placed in this process, by real path, each with the identity the folder
 * had then ({@link folderIdentity}): Git read that repository's format and took its folders. A repository made later at
 * the same path is another folder, which Git has not placed.
 */
const gitPlacedCommons = new Map<string, string>();

/**
 * A folder's identity on its volume: its file id and when it was made, so a folder removed and made again at its path is
 * another one (Linux file systems give a freed file id out again; NTFS keeps a name's creation time for a while after the
 * name is removed). Undefined when it cannot be read.
 */
function folderIdentity(folder: string): string | undefined {
  try {
    const stats = lstatSync(folder);
    return `${stats.ino}:${stats.birthtimeMs}`;
  } catch {
    return undefined;
  }
}

/** Whether Git placed a checkout of the repository whose common Git folder is `common`, a real path, as it is now. */
function gitPlaced(common: string): boolean {
  const placed = gitPlacedCommons.get(common);
  return placed !== undefined && placed === folderIdentity(common);
}

/**
 * Where `workspace`'s clone id lives: {@link GIT_IDENTITY_FILE} in Git's common metadata directory, shared by every
 * linked worktree, or `.rerail/repository-id` outside Git (or when Git is not installed). A folder that holds a `.git`
 * entry is answered as Git last answered while what Git reads there is unchanged, and a linked worktree of a repository
 * Git placed a checkout of is answered from its files ({@link linkedWorktreeFromFiles}); every other folder, and every
 * case those files do not decide, is asked of Git.
 */
export function repositoryIdentityLocation(workspace: string): RepositoryIdentityLocation {
  return placed(resolve(workspace), false);
}

/**
 * {@link repositoryIdentityLocation} of `requested`. A kept answer serves while its stamp is unchanged, and for folders Git
 * does not take for the user's only when `lastAnswer` is set: Git takes those through `safe.directory`, so each placement
 * of one asks Git, and Git's last acceptance there serves the worktree registry until the next. The registry
 * (`lastAnswer`) also takes Git's acceptances of folders the files cannot describe ({@link keptForRegistry}). A refusal
 * is never kept: the person may give their trust back at any time, and Git then lists the folder.
 */
function placed(requested: string, lastAnswer: boolean): RepositoryIdentityLocation {
  const kept = lastAnswer ? keptForRegistry(requested) : undefined;
  if (kept !== undefined) return kept;
  const inputs = placementInputs(requested);
  return inputs === undefined ? acceptedByGit(requested) : placedFromInputs(requested, inputs, lastAnswer);
}

/** {@link placed} for a folder whose placement inputs were read: the kept answer, else the files' or Git's. */
function placedFromInputs(requested: string, inputs: PlacementInputs, lastAnswer: boolean): RepositoryIdentityLocation {
  const held = heldTopLevels.get(requested);
  if (held?.stamp === inputs.stamp && (inputs.owned || lastAnswer)) return { ...held.location };
  const fromFiles = linkedLocation(inputs);
  const located = fromFiles === undefined ? locatedByGit(requested) : { location: fromFiles, byGit: false, searched: [] };
  holdTopLevel(requested, inputs, located);
  return { ...located.location };
}

/** An answer for a folder, and whether Git gave it (else the folder's files did). */
interface Located {
  readonly location: RepositoryIdentityLocation;
  readonly byGit: boolean;
  /** For an answer Git gave, what Git's search reads in each folder it may walk, read before Git ran ({@link searchedUp}). */
  readonly searched: readonly Searched[];
}

function locatedByGit(requested: string): Located {
  const searched = searchedUp(requested);
  const result = locateInGit(requested);
  const location = result === undefined || result.status !== 0 ? nonGitLocation(requested) : gitLocation(requested, result.stdout);
  return { location, byGit: true, searched };
}

/** A folder Git's search walks, and what it holds there ({@link searchedIn}). */
interface Searched {
  readonly folder: string;
  readonly entries: string;
}

/**
 * What Git's search for a repository reads in each folder it walks up from `requested`'s real path, the folder Git
 * changes into, as far as Git's search goes: up to the first folder where it ends ({@link searchStep}), within the
 * ceilings and the device ({@link foldersGitSearches}); none when the real path cannot be read. Read before Git runs: a
 * folder that becomes a repository while Git runs, after Git passed it, then moves what the next ask reads.
 */
function searchedUp(requested: string): Searched[] {
  const real = realOf(requested);
  const searched: Searched[] = [];
  for (const folder of real === undefined ? [] : foldersGitSearches(real)) {
    const step = searchStep(folder);
    searched.push({ folder, entries: step.entries });
    if (step.ends) break;
  }
  return searched;
}

function linkedLocation(inputs: PlacementInputs): RepositoryIdentityLocation | undefined {
  const placed = linkedWorktreeFromFiles(inputs, gitPlaced);
  if (placed === undefined) return undefined;
  return { git: true, workspaceRoot: placed.top, identityPath: join(placed.commonDirectory, GIT_IDENTITY_FILE), main: false };
}

/**
 * Keeps `located` for `folder` under the stamp of `inputs`, read before it was placed, when it names the folder as its
 * own top level ({@link answersInputs}): what makes that so is read at the folder and the Git folders it names, all in
 * the stamp. Any other answer depends on the folders above too, and is not kept for placements; Git's acceptance naming
 * a folder above is kept for the worktree registry under the same stamp and what Git's search read in the folders up to
 * that one ({@link keepAcceptance}). For folders Git does
 * not take for the user's, whose answer depends on the `safe.directory` entries of the user's and the system's Git
 * config too, Git's acceptance is kept for the worktree registry alone ({@link placed}).
 */
function holdTopLevel(folder: string, inputs: PlacementInputs, located: Located): void {
  heldTopLevels.delete(folder);
  gitAcceptances.delete(folder);
  try {
    if (answersInputs(folder, inputs, located)) keepTopLevel(folder, inputs, located);
    else if (located.byGit && !namesItself(folder, located.location)) keepAcceptance(folder, inputs.stamp, located);
  } catch {
    // A folder whose real path cannot be read now is asked of Git again next time.
  }
}

/** Keeps `located`, an answer naming `folder` as its own top level, and when Git gave it, that Git placed its repository. */
function keepTopLevel(folder: string, inputs: PlacementInputs, located: Located): void {
  // The answer names the common Git folder the inputs name ({@link answersInputs}): its stored form is read there.
  heldTopLevels.set(folder, { stamp: inputs.stamp, location: located.location, common: inputs.realCommon });
  if (located.byGit) rememberGitPlaced(storedPath(dirname(located.location.identityPath)));
}

/**
 * Whether `located` names `folder` as its own top level, and is the answer `inputs` describe. An answer read from the
 * files is. Git reads the files again as it runs, so a file rewritten while Git ran and back before the next read
 * would keep Git's answer about the other text under this stamp: Git's answer is kept only when it names the common
 * Git folder the inputs name, as a main checkout exactly when they name no other.
 */
function answersInputs(folder: string, inputs: PlacementInputs, located: Located): boolean {
  return namesItself(folder, located.location) && (!located.byGit || namesInputsCommon(located.location, inputs));
}

/** Whether `location` is in Git and names `folder` as its own top level; false when a real path cannot be read. */
function namesItself(folder: string, location: RepositoryIdentityLocation): boolean {
  try {
    return location.git && physicalPath(location.workspaceRoot) === physicalPath(folder);
  } catch {
    return false;
  }
}

function namesInputsCommon(location: RepositoryIdentityLocation, inputs: PlacementInputs): boolean {
  const main = inputs.linkedCommon === undefined;
  return storedPath(dirname(location.identityPath)) === inputs.realCommon && location.main === main;
}

function rememberGitPlaced(common: string): void {
  const identity = folderIdentity(common);
  if (identity !== undefined) gitPlacedCommons.set(common, identity);
}

/**
 * {@link repositoryIdentityLocation} without blocking: its one Git process runs off the thread, so a daemon goes on
 * answering hooks meanwhile. Once `signal` is aborted, that Git ends (or never starts) and the folder reads as outside
 * Git.
 */
export async function repositoryIdentityLocationAsync(workspace: string, signal?: AbortSignal): Promise<RepositoryIdentityLocation> {
  const requested = resolve(workspace);
  const result = outsideGit(requested) ? undefined : await runGitAsync(requested, LOCATE_ARGS, { signal }).catch(noGitInstalled);
  return result === undefined || result.status !== 0 ? nonGitLocation(requested) : gitLocation(requested, result.stdout);
}

/** Undefined when Git is not installed; any other failure is thrown again. */
function noGitInstalled(error: unknown): undefined {
  if (error instanceof GitUnavailableError) return undefined;
  throw error;
}

/**
 * Git's top level, common directory and Git directory for `requested`; undefined when Git is not installed, or when Git
 * would find no repository there ({@link outsideGit}), so a folder outside Git, asked about on every hook event of a
 * session working in it, starts no Git process.
 */
function locateInGit(requested: string): ReturnType<typeof runGit> | undefined {
  if (outsideGit(requested)) return undefined;
  try {
    return runGit(requested, LOCATE_ARGS);
  } catch (error) {
    return noGitInstalled(error);
  }
}

/**
 * What Git's search for a repository finds in a folder: a `.git` folder, file or link, or the `HEAD` every Git folder
 * holds (a bare repository, a folder inside a `.git`, a Git folder kept apart from its work tree).
 */
const GIT_MARKERS = [".git", "HEAD"];

/**
 * Whether Git, asked about `folder`, finds no repository, read from the disk without starting Git: `GIT_DIR` is unset in
 * the environment Rerail runs Git in, and no folder from `folder` up to its root holds one of {@link GIT_MARKERS}, by
 * the path as written and by its real path (Git searches up from the folder it changed into, and whether that folder's
 * name keeps a link's path or resolves it differs by system). False wherever the disk cannot tell: a folder that does
 * not resolve, an entry that cannot be read. So it skips Git only where Git has nothing to find; where a `.git` is there
 * and Git still answers no repository (another owner's, past `GIT_CEILING_DIRECTORIES`, an empty `.git` folder), Git is
 * asked as before.
 */
export function outsideGit(folder: string): boolean {
  if (process.env.GIT_DIR !== undefined) return false;
  const written = resolve(folder);
  let real: string;
  try {
    real = checkoutRealpath(written);
  } catch {
    return false;
  }
  return noMarkerUpFrom(written) && (real === written || noMarkerUpFrom(real));
}

/** Whether no folder from `start` up to its root holds one of {@link GIT_MARKERS}, or an entry that cannot be read. */
function noMarkerUpFrom(start: string): boolean {
  for (let current = start; ; current = dirname(current)) {
    if (GIT_MARKERS.some(name => entryMayBeThere(join(current, name)))) return false;
    if (dirname(current) === current) return true;
  }
}

/** Whether `path` names an entry of any kind, a broken link included; true when it cannot be read either way. */
function entryMayBeThere(path: string): boolean {
  try {
    return lstatSync(path, { throwIfNoEntry: false }) !== undefined;
  } catch {
    return true;
  }
}

function gitLocation(requested: string, stdout: string): RepositoryIdentityLocation {
  const [top, commonDirectory, gitDirectory] = stdout.trim().split("\n").map(line => resolve(line.trim()));
  if (top === undefined || commonDirectory === undefined || gitDirectory === undefined) {
    throw new Error(`could not resolve Git common directory for ${requested}`);
  }
  return { git: true, workspaceRoot: top, identityPath: join(commonDirectory, GIT_IDENTITY_FILE), main: commonDirectory === gitDirectory };
}

/**
 * Each common directory's Git registry, as `git worktree list` last gave it, and what the registry then held. Every
 * checkout of a repository reads the same registry, so a start that lists it from each session's checkout lists it once.
 */
const worktreeRegistries = new Map<string, WorktreeRegistry & { readonly contents: string }>();

export interface WorktreeRegistry {
  /** The common directory Git reads for the checkout that was asked, physical. */
  readonly commonDirectory: string;
  /** The main checkout and each linked worktree `git worktree add` made, as Git reports them. */
  readonly worktrees: readonly string[];
}

/**
 * Each registered worktree's name and where it says it is: `git worktree add`, `remove`, `move` and `repair` all change
 * it. The `worktrees` folder's own time would miss the last two, which rewrite a file inside an entry.
 */
function registryContents(commonDirectory: string): string {
  const folder = join(commonDirectory, "worktrees");
  let names: string[];
  try {
    names = readdirSync(folder);
  } catch {
    return "";
  }
  return names.toSorted().map(name => {
    try {
      return `${name}\0${readFileSync(join(folder, name, "gitdir"), "utf8")}`;
    } catch {
      return name;
    }
  }).join("\0");
}

/**
 * The registry of `checkout`, a recorded checkout: its own common directory and what Git lists there. Undefined when Git
 * cannot answer in it (it is gone, say). A `.git` file naming that directory is not listed, nor is a worktree moved
 * without `git worktree move` or `repair`. Git's answer is kept until the registry's contents change, so `.git` files a
 * repository plants cost Git one call, not one per file per tool call; only the recorded checkout decides what is kept,
 * never a folder asking to be listed. Line output because `-z` needs Git 2.36, which distributions Rerail runs on do not
 * have yet.
 */
export function worktreeRegistry(checkout: string): WorktreeRegistry | undefined {
  const registry = registryAsPlaced(checkout);
  if (registry !== ANOTHER_REGISTRY) return registry;
  // Git listed another repository's registry than the kept acceptance named: the checkout is placed again, by Git.
  gitAcceptances.delete(resolve(checkout));
  const again = registryAsPlaced(checkout);
  return again === ANOTHER_REGISTRY ? undefined : again;
}

/** What {@link registryAsPlaced} gives when Git listed the registry of another common Git folder than the one placed. */
const ANOTHER_REGISTRY = Symbol("another registry");

/** {@link worktreeRegistry} of `checkout` as it is placed now, the kept acceptance included. */
function registryAsPlaced(checkout: string): WorktreeRegistry | undefined | typeof ANOTHER_REGISTRY {
  const place = registryPlace(checkout);
  if (place === undefined) return undefined;
  const { commonDirectory } = place;
  // Read before Git runs: a worktree added meanwhile changes it, and Git is asked again next time.
  const contents = registryContents(commonDirectory);
  const known = storedList(place, contents);
  if (known !== undefined) return known;
  const registry = listWorktrees(checkout, commonDirectory, contents);
  // A checkout where Git cannot answer keeps nothing: what is kept is bounded by what answers.
  if (registry === undefined) forget(checkout, commonDirectory);
  // Every checkout of the repository reads what is stored here, so only a list of this registry is.
  else if (!listsRegistryOf(registry)) return ANOTHER_REGISTRY;
  else keepList(place, registry);
  return registry;
}

/**
 * Stores Git's list of the registry `place` names. Once its contents moved since the last list stored (a worktree
 * added, removed, moved or pruned), what is kept for the repository's folders that are gone leaves ({@link forgetGone}).
 */
function keepList(place: RegistryPlace, registry: WorktreeRegistry & { readonly contents: string }): void {
  if (worktreeRegistries.get(place.commonDirectory)?.contents !== registry.contents) forgetGone(storedForm(place.commonDirectory));
  worktreeRegistries.set(place.commonDirectory, registry);
}

/**
 * Drops each answer kept for a folder whose answer names `common`, a common Git folder in its {@link storedForm}, once
 * the folder is gone, and the config facts kept for files in `common` that are gone: a removed worktree's answers, its
 * own and those of the folders in it, leave with it, and a checkout that stays keeps its own. Each answer is matched by
 * the stored form of its common Git folder, read when it was kept, since an answer Git printed and one read from the
 * files can spell that folder two ways (on macOS a letter case, on Windows a drive letter's case or an 8.3 short
 * name), and a config file's path is spelt from the folder as asked. No ask takes an answer kept for
 * a folder that is gone ({@link registryPlace}, {@link placed}), so none that leaves would have served again. One
 * existence check per answer kept for the repository, and none for another repository's: a share that stops answering
 * holds only the lists of its own repositories, whose folders Git's list reads too.
 */
function forgetGone(common: string): void {
  for (const kept of [heldTopLevels, gitAcceptances]) forgetGoneFrom(kept, common);
  forgetGoneConfigs(common);
}

/** Drops from `kept` each answer whose common Git folder is `common` once its folder is gone ({@link forgetGone}). */
function forgetGoneFrom(kept: Map<string, { readonly common: string }>, common: string): void {
  for (const [folder, answer] of kept) if (answer.common === common && !existsSync(folder)) kept.delete(folder);
}

/** How many answers each map keeps: for tests, which count what a removed worktree leaves behind. */
export function keptAnswerCounts(): { readonly heldTopLevels: number; readonly gitAcceptances: number; readonly gitPlacedCommons: number } {
  return { heldTopLevels: heldTopLevels.size, gitAcceptances: gitAcceptances.size, gitPlacedCommons: gitPlacedCommons.size };
}

/**
 * The list stored for the registry `place` names while its contents are `contents`. None for a checkout placed by an
 * acceptance kept with no stamp: no fact ends that acceptance, so Git lists the registry at each ask and
 * {@link listsRegistryOf} judges the list.
 */
function storedList(place: RegistryPlace, contents: string): WorktreeRegistry | undefined {
  const known = worktreeRegistries.get(place.commonDirectory);
  return known !== undefined && known.contents === contents && !place.unstamped ? known : undefined;
}

/**
 * Whether Git listed the registry of `registry.commonDirectory`, read from Git's own list: Git names a registry's main
 * worktree after its common Git folder, as that folder itself (a bare repository, a Git folder kept apart, a `.git`
 * link) or as the folder holding it as `.git`, whatever `core.worktree` says. A list Git gave of another repository's
 * registry, where an acceptance outlived Git's answer, names another main worktree.
 */
function listsRegistryOf(registry: WorktreeRegistry): boolean {
  const [main] = registry.worktrees;
  try {
    return main !== undefined && [main, join(main, ".git")].some(path => physicalPath(path) === registry.commonDirectory);
  } catch {
    return false;
  }
}

/** Drops the registry of `commonDirectory`, and the acceptance of `checkout` kept for it, once Git cannot list there. */
function forget(checkout: string, commonDirectory: string): void {
  worktreeRegistries.delete(commonDirectory);
  gitAcceptances.delete(resolve(checkout));
}

/** Where a checkout's registry is, and whether Git's acceptance that placed the checkout is kept with no stamp. */
interface RegistryPlace {
  /** The physical common directory Git reads for the checkout. */
  readonly commonDirectory: string;
  readonly unstamped: boolean;
}

/**
 * The physical common directory Git reads for `checkout` now; undefined when the folder is gone or not in Git. Placed at
 * every call, and a placement is kept only while what Git reads at the folder is unchanged: a recorded checkout's folder
 * can be removed and become another repository's checkout, the folder above it renamed, or its `.git` file rewritten to
 * name another repository, and the registry to read is then that repository's. A kept placement costs a few lstats,
 * reads of small files and two real paths. For folders another account owns, Git's last acceptance; for folders the
 * files cannot describe, and while the environment moves Git's lookup, Git's acceptance kept for the registry
 * ({@link keptForRegistry}). A folder that is gone keeps no acceptance, as the registry kept nothing for it before: one
 * made again at its path is asked of Git.
 */
function registryPlace(checkout: string): RegistryPlace | undefined {
  const requested = resolve(checkout);
  if (!existsSync(checkout)) {
    gitAcceptances.delete(requested);
    return undefined;
  }
  const location = placed(requested, true);
  return location.git ? { commonDirectory: physicalPath(dirname(location.identityPath)), unstamped: keptUnstamped(requested) } : undefined;
}

/**
 * Whether the acceptance that placed `requested` is kept with no stamp: one kept so is always taken ({@link
 * keptForRegistry}), so one there is the one in force.
 */
function keptUnstamped(requested: string): boolean {
  const kept = gitAcceptances.get(requested);
  return kept !== undefined && kept.stamp === undefined;
}

/**
 * Git's acceptances of recorded checkouts that the worktree registry, asked at every tool call, takes in place of asking
 * Git again: of folders the files cannot describe ({@link gitEntryFacts}), of every folder while the environment moves
 * Git's lookup, and of folders Git places in a repository above them. Each is kept under the facts of the folder's own
 * `.git` and what Git's search reads in each folder from it up to the top level Git named (`searched`, {@link
 * keepAcceptance}), or, when the folder's own facts cannot be read, with no stamp: the registry then has Git list it at
 * each ask ({@link storedList}). A refusal is not kept. Each holds the answer's common Git folder in its {@link
 * storedForm} too, read once Git answered ({@link forgetGone}).
 */
const gitAcceptances = new Map<string, {
  readonly stamp: string | undefined;
  readonly searched: readonly string[];
  readonly location: RepositoryIdentityLocation;
  readonly common: string;
}>();

/** Git's acceptance of `requested` kept for the worktree registry while the facts it was kept under are unchanged. */
function keptForRegistry(requested: string): RepositoryIdentityLocation | undefined {
  const kept = gitAcceptances.get(requested);
  if (kept === undefined) return undefined;
  const now = stampOf(ownFacts(requested), kept.searched.map(searchedIn));
  return kept.stamp === undefined || kept.stamp === now ? { ...kept.location } : undefined;
}

/**
 * The facts of `requested`'s own `.git` a kept acceptance holds: its placement inputs read from the files ({@link
 * filePlacementInputs}), else the raw facts Git reads there ({@link gitEntryFacts}). Undefined when neither can be read.
 */
function ownFacts(requested: string): string | undefined {
  return filePlacementInputs(requested)?.stamp ?? gitEntryFacts(requested);
}

/**
 * A kept acceptance's stamp: `facts`, what Git's search reads in each folder it walked (`searched`), and the lookup
 * variables' values, which do not change while the process runs. Undefined when the facts could not be read.
 */
function stampOf(facts: string | undefined, searched: readonly string[]): string | undefined {
  return facts === undefined ? undefined : [facts, ...searched, lookupEnvironment() ?? ""].join("\n");
}

/**
 * Git's placement of `requested`, a folder the files cannot describe or one placed while the environment moves Git's
 * lookup, kept for the worktree registry under the facts read before Git ran.
 */
function acceptedByGit(requested: string): RepositoryIdentityLocation {
  heldTopLevels.delete(requested);
  const facts = ownFacts(requested);
  const located = locatedByGit(requested);
  keepAcceptance(requested, facts, located);
  return { ...located.location };
}

/**
 * Keeps Git's acceptance `located` of `requested` for the worktree registry under `facts`, the facts of the folder's own
 * `.git`, and what Git's search read in each folder from `requested` up to the top level Git named ({@link upToTop}),
 * all read before Git ran. One that names a folder above ends once the folder's own `.git` moves (`git init`, a clone or
 * a restore there makes one Git reads first), once a folder between becomes a repository (an empty `.git` folder there
 * filled in place included), and once the top level's own `.git` moves or, as a file, is rewritten in place. A refusal
 * is not kept.
 */
function keepAcceptance(requested: string, facts: string | undefined, located: Located): void {
  if (!located.location.git) {
    gitAcceptances.delete(requested);
    return;
  }
  const searched = upToTop(located.searched, located.location.workspaceRoot);
  const stamp = stampOf(facts, searched.map(folder => folder.entries));
  const common = storedForm(dirname(located.location.identityPath));
  gitAcceptances.set(requested, { stamp, searched: searched.map(folder => folder.folder), location: located.location, common });
}

/**
 * The folders of `searched` from the one asked up to `top`, the top level Git named; all of them, as far as Git's search
 * goes, when `top` is not among them (a `core.worktree` naming a folder elsewhere), since Git searched up from the
 * folder asked.
 */
function upToTop(searched: readonly Searched[], top: string): readonly Searched[] {
  const end = searched.findIndex(folder => folder.folder === top);
  return end === -1 ? searched : searched.slice(0, end + 1);
}

function listWorktrees(checkout: string, commonDirectory: string, contents: string): WorktreeRegistry & { readonly contents: string } | undefined {
  const result = runGit(checkout, ["worktree", "list", "--porcelain"]);
  if (result.status !== 0) return undefined;
  const worktrees = result.stdout.split("\n").filter(line => line.startsWith("worktree ")).map(line => line.slice("worktree ".length));
  return { commonDirectory, worktrees, contents };
}

/** The repository's own folder name: the main checkout's folder for a linked worktree, else the checkout's folder. */
export function repositoryFolderName(location: RepositoryIdentityLocation): string {
  const commonDirectory = dirname(location.identityPath);
  return location.git && basename(commonDirectory) === ".git" ? basename(dirname(commonDirectory)) : basename(location.workspaceRoot);
}

/** {@link repositoryFolderName} of the checkout `workspace` is in. */
export function repositoryName(workspace: string): string {
  return repositoryFolderName(repositoryIdentityLocation(workspace));
}

function errorCode(error: unknown): string | undefined {
  return typeof error === "object" && error !== null && "code" in error && typeof error.code === "string" ? error.code : undefined;
}

/** The clone id `workspace` already has, without creating one; passive readers use it. */
export function readCloneId(workspace: string): string | undefined {
  return readCloneIdAt(repositoryIdentityLocation(workspace).identityPath);
}

/**
 * A clone id: `git:<uuid>` or `<uuid>` today; ids written before (and `rerail.repository-id` config values) are
 * short tokens of letters, digits, `.`, `_`, `:` and `-`. Anything else is refused rather than trusted.
 */
const CLONE_ID = /^[A-Za-z0-9][A-Za-z0-9._:-]{0,99}$/;
const MAX_IDENTITY_BYTES = 128;

/**
 * The identity file's id, read without following a symlink, only from a regular file, and at most
 * {@link MAX_IDENTITY_BYTES}; undefined when it does not exist. Throws on a symlink, a non-file or a malformed id.
 */
function readIdentityFile(identityPath: string): string | undefined {
  let descriptor: number;
  try {
    descriptor = openNoFollowSync(identityPath, fsConstants.O_RDONLY);
  } catch (error) {
    if (errorCode(error) === "ENOENT") return undefined;
    throw new Error(`Rerail repository identity at ${identityPath} is not a regular file`);
  }
  try {
    if (!fstatSync(descriptor).isFile()) throw new Error(`Rerail repository identity at ${identityPath} is not a regular file`);
    const buffer = Buffer.alloc(MAX_IDENTITY_BYTES);
    const length = readSync(descriptor, buffer, 0, MAX_IDENTITY_BYTES, 0);
    const cloneId = buffer.subarray(0, length).toString("utf8").trim();
    if (!CLONE_ID.test(cloneId)) throw new Error(`malformed Rerail repository identity at ${identityPath}`);
    return cloneId;
  } finally {
    closeSync(descriptor);
  }
}

/** The clone id stored at `identityPath`, or undefined before one is created or when the file can't be trusted. */
export function readCloneIdAt(identityPath: string): string | undefined {
  try {
    return readIdentityFile(identityPath);
  } catch {
    return undefined;
  }
}

/** Publish complete identity bytes with an atomic no-clobber link. */
function readOrCreateIdentity(identityPath: string, candidate: string): string {
  const existing = readIdentityFile(identityPath);
  if (existing !== undefined) return existing;
  const temporaryPath = `${identityPath}.${process.pid}.${randomUUID()}.tmp`;
  writeFileSync(temporaryPath, `${candidate}\n`, { encoding: "utf8", flag: "wx", mode: 0o600 });
  try {
    try {
      linkSync(temporaryPath, identityPath);
    } catch (error) {
      if (errorCode(error) !== "EEXIST") throw error;
    }
  } finally {
    unlinkSync(temporaryPath);
  }
  const cloneId = readIdentityFile(identityPath);
  if (cloneId === undefined) throw new Error(`Rerail repository identity at ${identityPath} disappeared`);
  chmodSync(identityPath, 0o600);
  return cloneId;
}

/**
 * The clone id of a checkout in Git: the identity file's, else a legacy `rerail.repository-id` config value when it is
 * well formed, else a new one. Git's config is read only while no identity file is there, since the file wins once it
 * is: so a repository's worktrees, which share one, start no Git process to be recorded.
 */
function gitCloneId(location: RepositoryIdentityLocation): string {
  const existing = readIdentityFile(location.identityPath);
  if (existing !== undefined) return existing;
  const legacyId = gitText(location.workspaceRoot, ["config", "--local", "--get", "rerail.repository-id"]) ?? "";
  return readOrCreateIdentity(location.identityPath, CLONE_ID.test(legacyId) ? legacyId : `git:${randomUUID()}`);
}

/**
 * `workspace`'s clone id, created on first use and stored atomically in Git's common metadata directory. Linked
 * worktrees share it; clones and forks receive a new one; a legacy `rerail.repository-id` Git config value is adopted
 * when it is a well-formed id. Folders outside Git use the same file contract under `.rerail`, which must not be a
 * symlink.
 */
export function resolveCloneId(workspace: string): { cloneId: string; location: RepositoryIdentityLocation } {
  const location = repositoryIdentityLocation(workspace);
  if (location.git) return { cloneId: gitCloneId(location), location };
  const directory = dirname(location.identityPath);
  // Created first, so two processes recording the same folder at once both succeed; whichever made it, it must be a
  // real directory, never a symlink (lstat).
  try {
    mkdirSync(directory, { mode: 0o700 });
  } catch (error) {
    if (errorCode(error) !== "EEXIST") throw error;
    if (!lstatSync(directory).isDirectory()) throw new Error(`${directory} is not a directory Rerail can keep its identity in`);
  }
  return { cloneId: readOrCreateIdentity(location.identityPath, randomUUID()), location };
}

export interface RemoteAddress {
  name: string;
  url: string;
}

/**
 * The checkout's `origin` remote, else its only remote; undefined with no remote or several and no `origin`. Git's
 * answer, "no remote" included, is kept while the configuration it comes from reads the same ({@link remoteSource}),
 * so a remote added later is found, and recording every worktree of a clone with no remote asks Git once, not twice
 * per worktree.
 */
export function readRemoteAddress(root: string): RemoteAddress | undefined {
  const source = remoteSource(root);
  const kept = source === undefined ? undefined : keptRemotes.get(source.key);
  if (kept !== undefined && kept.facts === source?.facts) return kept.remote;
  const answers: GitResult[] = [];
  for (let step = remoteStep(answers); "ask" in step; step = remoteStep(answers)) answers.push(runGit(root, step.ask));
  return keepRemote(source, answers);
}

/** A remote Git named, "none" included, and the facts it rested on. */
interface KeptRemote {
  readonly facts: string;
  readonly remote: RemoteAddress | undefined;
}

/**
 * Each clone's remote, by the Git folder its remotes are read from: the common one, which every worktree of the clone
 * shares, or a worktree's own when it has a `config.worktree`.
 */
const keptRemotes = new Map<string, KeptRemote>();

interface RemoteSource {
  readonly key: string;
  readonly facts: string;
}

/**
 * Where `root`'s remotes come from, read now: the repository's `config`, the worktree's `config.worktree`, and the
 * person's global files, `GIT_CONFIG_GLOBAL` when set, else `.gitconfig` in Git's home ({@link gitHome}) and `git/config`
 * in `XDG_CONFIG_HOME` or that home's `.config` (an `insteadOf` there rewrites an address), each by its text. The
 * system file, which only an administrator writes, is left out, as for the excludes files Rerail reads. Undefined, so
 * Git is asked each time, where Git's placement of `root` is not read from its files ({@link placementInputs}), where
 * another account owns it (Git reads the repository's config only while a `safe.directory` entry trusts it, the
 * system file's too), or where a file cannot be read or includes another.
 */
function remoteSource(root: string): RemoteSource | undefined {
  const inputs = placementInputs(resolve(root));
  if (inputs === undefined || !inputs.owned) return undefined;
  const own = configText(join(inputs.gitFolder, "config.worktree"));
  const files = [join(inputs.linkedCommon ?? inputs.gitFolder, "config"), ...globalConfigFiles()];
  const texts = [own, ...files.map(configText)];
  if (texts.some(text => text === undefined)) return undefined;
  const key = own === NO_FILE ? inputs.realCommon : inputs.gitFolder;
  return { key, facts: [...files, ...texts].join("\0") };
}

function globalConfigFiles(): string[] {
  const named = process.env["GIT_CONFIG_GLOBAL"];
  if (named !== undefined && named !== "") return [named];
  const xdg = process.env["XDG_CONFIG_HOME"];
  return [join(gitHome(), ".gitconfig"), join(xdg === undefined || xdg === "" ? join(gitHome(), ".config") : xdg, "git", "config")];
}

const NO_FILE = "\0none";
/** A config section that reads another file, which this source does not follow. */
const INCLUDE = /^\s*\[\s*include/imu;
/**
 * A config file's text, read now as Git reads it ({@link fileText}); {@link NO_FILE} when nothing is there; undefined
 * when it is no regular file, cannot be read or includes another.
 */
function configText(path: string): string | undefined {
  const read = fileText(path);
  if (read === "none") return NO_FILE;
  return read === undefined || INCLUDE.test(read.text) ? undefined : read.text;
}

/** The next Git question on the way to a checkout's remote, given the answers so far, or the remote they give. */
type RemoteStep = { readonly ask: readonly string[] } | { readonly remote: RemoteAddress | undefined };

/** Trimmed stdout of an answer Git gave, or undefined when it failed. */
function textOf(answer: GitResult): string | undefined {
  return answer.status === 0 ? answer.stdout.trim() : undefined;
}

/**
 * `origin`'s address (`ls-remote --get-url` gives the name back when no remote has it), else the address of the only
 * remote `git remote` lists.
 */
function remoteStep(answers: readonly GitResult[]): RemoteStep {
  const [origin, names, url] = answers;
  if (origin === undefined) return { ask: ["ls-remote", "--get-url", "origin"] };
  const originUrl = textOf(origin);
  if (originUrl !== undefined && originUrl !== "" && originUrl !== "origin") return { remote: { name: "origin", url: originUrl } };
  if (names === undefined) return { ask: ["remote"] };
  return soleRemoteStep(names, url);
}

function soleRemoteStep(names: GitResult, url: GitResult | undefined): RemoteStep {
  const remotes = (textOf(names) ?? "").split("\n").map(name => name.trim()).filter(name => name.length > 0);
  if (remotes.length !== 1) return { remote: undefined };
  const [name] = remotes as [string];
  if (url === undefined) return { ask: ["remote", "get-url", "--", name] };
  const address = textOf(url);
  return { remote: address === undefined || address === "" ? undefined : { name, url: address } };
}

/**
 * The remote the answers give, kept under the source's facts only when Git answered every question: a Git that failed
 * answers no remote, as before, and is asked again next time.
 */
function keepRemote(source: RemoteSource | undefined, answers: readonly GitResult[]): RemoteAddress | undefined {
  const step = remoteStep(answers);
  const remote = "remote" in step ? step.remote : undefined;
  if (source !== undefined && answers.every(answer => answer.status === 0)) keptRemotes.set(source.key, { facts: source.facts, remote });
  return remote;
}
const SCHEME = /^[A-Za-z][A-Za-z0-9+.-]*:\/\//;
const SCP_FORM = /^(?:[^@/]+@)?([^:/]+):(.*)$/;

function splitAddress(address: string): { host: string; path: string } | undefined {
  return SCHEME.test(address) ? splitUrl(address) : splitScpForm(address);
}

/** A `scheme://[user@]host[:port]/path` address; a `file://` one is a local path. */
function splitUrl(address: string): { host: string; path: string } | undefined {
  if (/^file:\/\//i.test(address)) return undefined;
  const rest = address.replace(SCHEME, "");
  const slash = rest.indexOf("/");
  const authority = slash === -1 ? rest : rest.slice(0, slash);
  const host = authority.slice(authority.lastIndexOf("@") + 1).replace(/:\d*$/, "");
  return { host, path: slash === -1 ? "" : rest.slice(slash) };
}

/** A `[user@]host:path` address. */
function splitScpForm(address: string): { host: string; path: string } | undefined {
  const scp = SCP_FORM.exec(address);
  // A single-letter "host" is a Windows drive (`C:/repo`, `C:\repo`), which is a local path.
  if (scp === null || scp[1]!.length === 1) return undefined;
  return { host: scp[1]!, path: scp[2]! };
}

/**
 * `remote:<host>/<path>` for a Git remote address, so the ssh, scp and https forms of one repository agree:
 * scheme, `user@` and port dropped, `//` collapsed, trailing `/`, `.git`, `/` stripped, lowercased. A `file://`
 * or local path gives undefined. An ssh `Host` alias stays its own host.
 */
export function normalizeRemoteAddress(url: string): RepositoryKey | undefined {
  const parts = splitAddress(url.trim());
  if (parts === undefined) return undefined;
  const path = parts.path.replace(/\/{2,}/g, "/").replace(/^\//, "").replace(/\/$/, "").replace(/\.git$/i, "").replace(/\/$/, "");
  if (parts.host === "" || path === "") return undefined;
  return `remote:${parts.host}/${path}`.toLowerCase();
}

/** The repository key: the normalised remote address when there is one, else `clone:<clone id>`. */
export function repositoryKeyOf(remote: RemoteAddress | undefined, cloneId: string): RepositoryKey {
  return (remote === undefined ? undefined : normalizeRemoteAddress(remote.url)) ?? `clone:${cloneId}`;
}
