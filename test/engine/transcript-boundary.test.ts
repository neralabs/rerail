import { type EncodingOption, mkdirSync, mkdtempSync, type PathLike, realpathSync, writeFileSync } from "node:fs";
import { homedir, hostname, tmpdir } from "node:os";
import { join, relative, resolve, sep } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  agentTranscriptPayload, callNamesDeniedPath, callNamesSecretPath, WITHHELD_INPUT, WITHHELD_OUTPUT, WITHHELD_PAYLOAD,
} from "../../src/engine/transcript-boundary.js";
import { linkFolder, symlinkOrSkip } from "../support/platform/links.js";
import { removeFolderSync } from "../support/platform/remove-folder.js";

const roots: string[] = [];
afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllEnvs();
  for (const root of roots.splice(0)) removeFolderSync(root);
});

/** An empty checkout folder, removed after the test. */
function checkout(prefix: string): string {
  const root = realpathSync(mkdtempSync(join(tmpdir(), prefix)));
  roots.push(root);
  return root;
}

/**
 * Calls that name URLs as agents do; 192.0.2.1 is a documentation address (RFC 5737) no host answers on, and
 * `archive.example`, `proxy.example` and `a.example` documentation names (RFC 2606). On Windows each URL's host and
 * path name a network share; on Linux and macOS they are a local path.
 */
const URL_INPUTS = [
  { command: "curl -s https://192.0.2.1/api/v1 | jq ." },
  { url: "https://192.0.2.1/docs/a.md" },
  { command: "git clone ssh://git@192.0.2.1:2222/team/app.git" },
  { command: "curl http://user:pw@192.0.2.1:8080/notes.txt?page=2" },
  // A scheme after a bracket, an emphasis mark, a list's number or dash, or in another URL's path: a Markdown link,
  // bold text, a list item with no space, an archived page, a proxy.
  { content: "See [https://192.0.2.1/api/v2](https://192.0.2.1/api/v2), **https://192.0.2.1/b** or _https://192.0.2.1/i_." },
  { content: "Links\n1.https://192.0.2.1/l/1\n-https://192.0.2.1/l/2" },
  { url: "https://archive.example/web/2020/https://192.0.2.1/docs/b.md" },
  { content: "Archived as archive.example/web/2020/https://192.0.2.1/docs/c.md" },
  { url: "http://localhost:8080/https://192.0.2.1/api/v3" },
  // A scheme after any other character that is no letter or digit: a comment with no space, curly quotes, guillemets,
  // full-width brackets, text in another script, a package spec, a URL in another's query or fragment, an HTML
  // comment, a percent sign.
  { content: "#https://192.0.2.1/s/1" },
  { content: "See “https://192.0.2.1/s/2”, «https://192.0.2.1/s/3» or （https://192.0.2.1/s/4）." },
  { content: "詳細はhttps://192.0.2.1/s/5を参照" },
  { command: "npm install pkg@https://192.0.2.1/s/6/pkg.tgz" },
  { url: "https://proxy.example/?https://192.0.2.1/s/7" },
  { url: "https://a.example/#https://192.0.2.1/s/8" },
  { content: "<!--https://192.0.2.1/s/9-->" },
  { content: "%https://192.0.2.1/s/10" },
  // A scheme after a digit or a percent-encoded character: a share link's text, a mail link's body, a URL glued to a
  // step's number or to a timestamp.
  { url: "https://share.example/intent?text=Read%20https://192.0.2.1/p/1" },
  { url: "mailto:a@mail.example?body=See%20https://192.0.2.1/p/2" },
  { content: "ステップ1https://192.0.2.1/p/3" },
  { content: "2026-11-13T00:00:00Zhttps://192.0.2.1/p/4" },
  // A placeholder for the scheme: a shell variable, a JavaScript template, a Python f-string, printf and format
  // strings; and nothing before the `:`.
  { command: "curl -s \"${API_SCHEME}://192.0.2.1/v1/users\"" },
  { content: "fetch(`${scheme}://192.0.2.1/p/5`)" },
  { content: "requests.get(f\"{scheme}://192.0.2.1/p/6\")" },
  { content: "printf(\"%s://192.0.2.1/p/7\", scheme)" },
  { content: "base = \"{}://192.0.2.1/p/8\"" },
  { content: "://192.0.2.1/p/9" },
  // Hand-written device paths whose `.` and `..` names, or a name's trailing period, hide the network redirector: the
  // lookup drops them (`path.resolve` the names, Windows the period after `\\.\`) and opens `\\.\UNC\192.0.2.1\d\1`.
  { content: "https://./../UNC/192.0.2.1/d/1" },
  { content: "https://././UNC/192.0.2.1/d/2" },
  { content: "https://./x/../../UNC/192.0.2.1/d/3" },
  { content: "https://?/GLOBALROOT/Device/x/../Mup/192.0.2.1/d/4" },
  { content: "https://./UNC./192.0.2.1/d/5" },
  // A scheme that only ends in the letters of `file` or `smb`, a word's: read as any other scheme's URL.
  { command: "curl -s profile://192.0.2.1/s/11" },
  { content: "See Makefile://192.0.2.1/s/12" },
];

/**
 * Calls that name another machine's share right after a `:` where it is no URL's host and path, written with
 * backslashes after a switch, one-letter switches too, whose letter is no drive's; or in an `smb:` or `file:` URL,
 * which names a share outright; each with the share as the lookup opens it.
 */
const SHARES_AFTER_A_COLON = [
  { form: "robocopy's log switch", command: "robocopy src dst /LOG:\\\\192.0.2.1\\logs\\run.txt", share: "\\\\192.0.2.1\\logs\\run.txt" },
  { form: "a PowerShell parameter", command: "Get-ChildItem -Path:\\\\192.0.2.1\\data\\cfg", share: "\\\\192.0.2.1\\data\\cfg" },
  { form: "a PowerShell parameter's one letter", command: "Get-ChildItem -P:\\\\192.0.2.1\\data\\cfg", share: "\\\\192.0.2.1\\data\\cfg" },
  { form: "a one-letter parameter after an en dash", command: "Get-ChildItem \u2013P:\\\\192.0.2.1\\data\\cfg", share: "\\\\192.0.2.1\\data\\cfg" },
  { form: "findstr's one-letter switch", command: "findstr /G:\\\\192.0.2.1\\data\\cfg notes.txt", share: "\\\\192.0.2.1\\data\\cfg" },
  { form: "findstr's switch glued to its name", command: "findstr/G:\\\\192.0.2.1\\data\\cfg notes.txt", share: "\\\\192.0.2.1\\data\\cfg" },
  { form: "an smb: URL", command: "curl -s smb://192.0.2.1/share/cfg", share: "\\\\192.0.2.1\\share\\cfg" },
  { form: "a file: URL", command: "curl -s file://192.0.2.1/share/cfg", share: "\\\\192.0.2.1\\share\\cfg" },
  { form: "a FILE: URL", command: "curl -s FILE://192.0.2.1/share/cfg", share: "\\\\192.0.2.1\\share\\cfg" },
  { form: "a git+file: URL", command: "git clone git+file://192.0.2.1/share/cfg", share: "\\\\192.0.2.1\\share\\cfg" },
];

/**
 * Calls that name URLs whose host and path land under the automount root `/net` on Linux and macOS, spelled as the
 * scheme rule there reads them for certain: plain, a Markdown link, bold, a numbered list item, a comment, curly
 * quotes; and through `..`, a `.` host or an empty host, which `path.resolve` takes to `/net` as well.
 */
const AUTOMOUNT_URL_INPUTS = [
  { command: "curl -s https://net/192.0.2.1/api/v1 | jq ." },
  { url: "https://net/192.0.2.1/docs/a.md" },
  { content: "See [https://net/192.0.2.1/api/v2](https://net/192.0.2.1/api/v2) or **https://net/192.0.2.1/b**." },
  { content: "1.https://net/192.0.2.1/l/1\n-https://net/192.0.2.1/l/2\n#https://net/192.0.2.1/c\n“https://net/192.0.2.1/q”" },
  { content: "<!--https://net/192.0.2.1/h-->" },
  { url: "https://a.example/../net/192.0.2.1/n/1" },
  { url: "https://./net/192.0.2.1/n/2" },
  { url: "https:///net/192.0.2.1/n/3" },
];

/**
 * A Windows device path (`\\?\`, `\\.\`) with no network redirector on its way, which stays on this machine. Windows
 * drops a name's trailing period after `\\.\`, so `\\.\UNC.\host` goes to the redirector too.
 */
const localDevice = (path: string): boolean =>
  /^\\\\[.?]\\/u.test(path) && !/\\(?:unc|mup|rdpdr|\w*redirector)\.?(?:\\|$)/iu.test(path);

/** A share on Windows, or a path under `/net` elsewhere, on the loopback host, which stays on this machine. */
const loopback = (path: string): boolean => /^(?:\\\\|\/net\/)(?:localhost|127(?:\.\d{1,3}){3})(?:[\\/]|$)/iu.test(path);

/** Whether a lookup would reach another machine: a network share on Windows, or a path under `/net` elsewhere. */
const reachesAnotherMachine = (path: string): boolean =>
  (path.startsWith("\\\\") || /^\/net\//iu.test(path)) && !localDevice(path) && !loopback(path);

/**
 * A checkout `repo`, with a session folder `repo/packages/app` in it, and a secret folder `home/.aws` beside it. Links
 * to that folder sit in the checkout (`repo/docs/cfg`), in the home folder (`home/cfg`), in the temp folder
 * (`temp/cfg`) and in another folder (`out/cfg`); for the rest of the test `homedir()` and `tmpdir()` name `home` and
 * `temp`. A link to a folder is a junction on Windows, which needs no privilege to make.
 */
function linksToSecretFolder(): { base: string; repo: string; app: string } {
  const base = checkout("rerail-transcript-links-");
  const repo = join(base, "repo");
  const app = join(repo, "packages", "app");
  const secrets = join(base, "home", ".aws");
  for (const folder of [app, join(repo, "docs"), join(base, "temp"), join(base, "out"), secrets]) mkdirSync(folder, { recursive: true });
  writeFileSync(join(secrets, "config"), "[default]\n");
  for (const folder of [join(repo, "docs"), join(base, "home"), join(base, "temp"), join(base, "out")]) {
    linkFolder(secrets, join(folder, "cfg"));
  }
  for (const name of ["HOME", "USERPROFILE"]) vi.stubEnv(name, join(base, "home"));
  for (const name of ["TMPDIR", "TEMP", "TMP"]) vi.stubEnv(name, join(base, "temp"));
  return { base, repo, app };
}

/** `path` as a URL's path, spelled as an agent writes one: `/home/me/a.txt`, `/C:/Users/me/a.txt`. */
const urlPath = (path: string): string => `${path.startsWith("/") ? "" : "/"}${path.split(sep).join("/")}`;

/**
 * Records each path `realpathSync.native` is asked for. A path in `links` is answered as a link to its target. Any other
 * path that would reach another machine, or the loopback host, is refused, not looked up: on Windows a `\\` path other
 * than a device path names a share, and the lookup waits about 21 s for a host that does not answer; under `/net` an
 * automounter may mount the named host's exports.
 */
function recordLookups(links: ReadonlyMap<string, string> = new Map()): string[] {
  const lookUp = realpathSync.native;
  const lookups: string[] = [];
  vi.spyOn(realpathSync, "native").mockImplementation((path: PathLike, options?: EncodingOption) => {
    lookups.push(String(path));
    const target = links.get(String(path));
    if (target !== undefined) return target;
    if (reachesAnotherMachine(String(path)) || loopback(String(path))) {
      throw Object.assign(new Error(`ENOENT: no such file or directory, realpath '${String(path)}'`), { code: "ENOENT" });
    }
    return lookUp(path, options);
  });
  return lookups;
}

/**
 * `path` on Windows spelled in the device namespace with no drive colon (`\\?\GLOBALROOT\Device\HarddiskVolume3\...`),
 * its volume found by looking up each spelling until one leads where `path` does.
 */
function onVolume(path: string): string {
  const target = realpathSync.native(path);
  for (let volume = 1; volume <= 64; volume += 1) {
    const spelled = `\\\\?\\GLOBALROOT\\Device\\HarddiskVolume${volume}${path.slice(2)}`;
    try {
      if (realpathSync.native(spelled) === target) return spelled;
    } catch {
      // No volume by that number, or the path is not on it.
    }
  }
  throw new Error(`no HarddiskVolume device holds ${path}`);
}

/** `main`'s delimiters around a word (`TOKEN_BREAK` at 821eac82e), which keeps none of them. */
const MAIN_TOKEN_BREAK = /[\s"'`|;&<>(),:={}]+/u;

/**
 * One character of each kind a word can hold, since {@link MAIN_TOKEN_BREAK} splits at none of them: an ASCII letter, a
 * digit, every ASCII mark that is no delimiter (both slashes, a dash, a dot, glob, bracket and other marks), an en dash
 * for the dashes PowerShell also takes, and a letter of another script.
 */
const WORD_CHARACTERS = ["a", "1", "/", "\\", "-", "–", ".", "*", "[", "]", "_", "+", "~", "%", "@", "#", "!", "?", "^", "$", "é"] as const;

/** Every string of `length` characters from {@link WORD_CHARACTERS}. */
const stringsOf = (length: number): string[] =>
  length === 0 ? [""] : stringsOf(length - 1).flatMap(start => WORD_CHARACTERS.map(character => `${start}${character}`));

/** What stands before a word and its `:` in the table of spellings: every string of 0 to 3 word characters. */
const BEFORE_A_WORD: readonly string[] = [0, 1, 2, 3].flatMap(stringsOf);

/** The words before the `:`: one letter, which may be a drive letter, a web scheme, and a word of two letters. */
const WORDS = ["C", "https", "ab"] as const;

/**
 * The separators right after the `:`, each with the one between the names after them: one backslash, two as JSON
 * carries one, four as JSON in JSON does, one slash, two slashes before a URL's host.
 */
const AFTER_A_COLON = [
  { lead: "\\", inner: "\\" }, { lead: "\\\\", inner: "\\\\" }, { lead: "\\\\\\\\", inner: "\\\\\\\\" },
  { lead: "/", inner: "/" }, { lead: "//", inner: "/" },
] as const;

/** The names after the separators: a folder, and a host under the automount root `/net` on Linux and macOS. */
const NAMES_AFTER = [["Users", "me"], ["net", "192.0.2.1", "me"]] as const;

/** One spelling of the table: `text`, built from `before`, `word`, the `:`, `lead` and the word `afterColon`. */
interface Spelling {
  readonly text: string;
  readonly before: string;
  readonly word: (typeof WORDS)[number];
  readonly lead: (typeof AFTER_A_COLON)[number]["lead"];
  readonly afterColon: string;
  readonly net: boolean;
}

/** Every spelling: each thing before a word, each word, each separator run, each set of names. */
function* spellings(): Generator<Spelling> {
  for (const before of BEFORE_A_WORD) {
    for (const word of WORDS) {
      for (const { lead, inner } of AFTER_A_COLON) {
        for (const names of NAMES_AFTER) {
          const afterColon = `${lead}${names.join(inner)}`;
          yield { text: `${before}${word}:${afterColon}`, before, word, lead, afterColon, net: names[0] === "net" };
        }
      }
    }
  }
}

/** The dashes PowerShell reads before a parameter: a hyphen-minus, an en dash, an em dash, a horizontal bar. */
const DASHES = ["-", "–", "—", "―"];

/**
 * Whether the description promises that a one-letter `word` after `before` is a drive letter on Windows: nothing stands
 * before it in its word, or a character that is no ASCII letter or digit (a word's), no dash and no slash (a switch's,
 * glued to other text or not: `-P`, `findstr/G`, `x-P`), or a slash after another slash (a `file:` URL's `///C`).
 */
function promisedDrive(before: string, word: string): boolean {
  const last = before.at(-1);
  if (word.length !== 1 || last === undefined) return word.length === 1;
  if (last === "/") return before.at(-2) === "/";
  return !/^[a-z\d]$/iu.test(last) && !DASHES.includes(last);
}

/**
 * Whether the description promises that `https` after `before` is for certain a web scheme on Linux and macOS: after a
 * list's number or none, nothing but marks before it in its word (no ASCII letter, digit or slash of either kind), and
 * the word does not start with `~` or `.`.
 */
function promisedWebScheme(before: string): boolean {
  const marks = before.replace(/^\d+\./u, "");
  return !/[a-z\d\\/]/iu.test(marks) && !/^[~.]/u.test(marks);
}

/** Whether `word` has a segment that is a name: not empty, not `.`, `..` or a run of dots, not a spread (`...x`). */
const hasName = (word: string): boolean =>
  word.split(/[\\/]+/u).some(segment => segment !== "" && !/^(?:\.+|\.{3}.*)$/u.test(segment));

/**
 * The words `main`'s check looks up in `text`, in order, each once. `textNamesPath` at 821eac82e splits the text at
 * {@link MAIN_TOKEN_BREAK}, drops brackets, and looks up as written every word with a letter and a name that is shaped
 * like a path (a separator, a leading `.` or `~`, or an extension). It reads no word's neighbours, so this restatement
 * stays `main`'s while the check under test changes.
 */
function mainLookups(text: string): string[] {
  const words = text.split(MAIN_TOKEN_BREAK).map(word => word.replace(/[[\]]/gu, ""));
  return [...new Set(words.filter(word => /[A-Za-z]/u.test(word) && hasName(word) && /[\\/]|^[.~]|\.[^.]+$/u.test(word)))];
}

const AS_ON_MAIN = "as on main";
const DRIVE_PATH = "a drive path: looked up as its single-separator spelling";
const URL_HOST = "a URL's host and path, which would reach another machine: not looked up";
const NET_URL = "a web URL under /net on another host: not looked up";
type Change = typeof AS_ON_MAIN | typeof DRIVE_PATH | typeof URL_HOST | typeof NET_URL;

/**
 * On Windows, what the description promises for the word after the `:`: only two separators then a name open a share
 * on another machine. After a drive letter that word is a drive path written with doubled separators; after any other
 * word, one written with `//` may be a URL's host and path; one written with backslashes is looked up as on `main`.
 */
function windowsChange({ before, word, lead }: Spelling): Change {
  if (lead !== "\\\\" && lead !== "//") return AS_ON_MAIN;
  if (promisedDrive(before, word)) return DRIVE_PATH;
  return lead === "//" ? URL_HOST : AS_ON_MAIN;
}

/** On Linux and macOS, what the description promises: a web URL's `//` host and path under `/net` is not looked up. */
const posixChange = ({ before, word, lead, net }: Spelling): Change =>
  word === "https" && lead === "//" && net && promisedWebScheme(before) ? NET_URL : AS_ON_MAIN;

/** The words the description promises are looked up for `spelling` on this system, from `main`'s and its `change`. */
function promisedLookups(spelling: Spelling, change: Change, main: readonly string[]): readonly string[] {
  if (change === AS_ON_MAIN) return main;
  const others = main.filter(word => word !== spelling.afterColon);
  return change === DRIVE_PATH ? [...others, spelling.afterColon.replace(/([\\/])[\\/]+/gu, "$1")] : others;
}

/** One spelling's three verdicts: `main`'s lookups, this check's, and the ones the description promises. */
interface Verdicts {
  readonly text: string;
  readonly change: Change;
  readonly main: readonly string[];
  readonly head: readonly string[];
  readonly promised: readonly string[];
}

/**
 * The verdicts of `spelling` under `change`: `main`'s lookups restated ({@link mainLookups}), this check's (`lookedUp`),
 * and the promised ones, each resolved in the checkout `root`.
 */
function verdictsOf(spelling: Spelling, change: Change, root: string, lookedUp: (text: string) => string[]): Verdicts {
  const mainWords = mainLookups(spelling.text);
  const promised = promisedLookups(spelling, change, mainWords).map(word => resolve(root, word));
  return { text: spelling.text, change, main: mainWords.map(word => resolve(root, word)), head: lookedUp(spelling.text), promised };
}

/** Whether this check's lookups differ from the promised ones, or reach a machine `main`'s lookups do not. */
const breaksPromise = ({ main, head, promised }: Verdicts): boolean =>
  head.join("\n") !== promised.join("\n") || head.some(path => reachesAnotherMachine(path) && !main.includes(path));

/**
 * The table's spellings counted by the change promised for each, and those that break the promise counted by the last
 * two characters before the word, the first 20 kept by what stands before the word.
 */
class Tally {
  readonly changes: Record<Change, number> = { [AS_ON_MAIN]: 0, [DRIVE_PATH]: 0, [URL_HOST]: 0, [NET_URL]: 0 };
  broken = 0;
  readonly byLastTwo: Record<string, number> = {};
  readonly first: Record<string, Verdicts[]> = {};

  add(before: string, verdicts: Verdicts): void {
    this.changes[verdicts.change] += 1;
    if (!breaksPromise(verdicts)) return;
    this.broken += 1;
    const lastTwo = `[${before.slice(-2)}]`;
    this.byLastTwo[lastTwo] = (this.byLastTwo[lastTwo] ?? 0) + 1;
    if (this.broken <= 20) (this.first[`[${before}]`] ??= []).push(verdicts);
  }
}

/**
 * Records each path `realpathSync.native` is asked for and answers none from disk: `root` is itself, and any other path
 * is missing, so the table's thousands of spellings read nothing on disk and reach no host.
 */
function recordLookupsOnly(root: string): string[] {
  const lookups: string[] = [];
  const missing = Object.assign(new Error("ENOENT: no such file or directory"), { code: "ENOENT" });
  vi.spyOn(realpathSync, "native").mockImplementation((path: PathLike) => {
    lookups.push(String(path));
    if (String(path) === root) return root;
    throw missing;
  });
  return lookups;
}

describe("the transcript boundary for coding agents", () => {
  it("finds a hidden or secret-like path wherever a call's input carries it", () => {
    for (const input of [
      { command: "cat .env" },
      { command: "cat ./.env.local | grep KEY" },
      { command: "ssh-keygen -y -f ~/.ssh/id_rsa" },
      { command: "type C:\\Users\\me\\.aws\\credentials" },
      { command: "curl --cacert=/etc/pki/server.pem https://x" },
      { file_path: "/repo/config/secrets.yaml" },
      { path: "/repo/src/.hidden/a.ts" },
      { pattern: "**/*.pem" },
      { edits: [{ path: "/repo/deploy/service-account-prod.json" }] },
      // A Kubernetes secret mount's `..data`, and any other name after `..`, is still a name.
      { command: "cat /mnt/secrets/..data/password" },
      { command: "cat ..x/.env" },
      { command: "cat ../.env" },
      // A brace or bracket glob still names what it expands to.
      { command: "cat {.env,.npmrc}" },
      { command: "cat src/{app.ts,.env}" },
      { command: "cat [.]env" },
      { command: "cat ${HOME}/.netrc" },
    ]) expect(callNamesDeniedPath(input), JSON.stringify(input)).toBe(true);
  });

  it("leaves ordinary source, dots, ellipses, numbers and prose alone", () => {
    for (const input of [
      { command: "cat src/app.ts" },
      { command: "cd .. && ls ./src" },
      { command: "git log --oneline -5" },
      { file_path: "/repo/src/password-validator.ts" },
      { content: "Wait...and then retry. Version 0.5 is out; see e.g. the notes..." },
      { new_string: "const view = { ...props, ...rest };" },
      { command: "npm test -- --grep 'env'" },
      { pattern: "**/*.ts" },
      "plain text input",
      42,
      null,
    ]) expect(callNamesDeniedPath(input), JSON.stringify(input)).toBe(false);
  });

  it("counts an input deeper or larger than it reads as naming a denied path", () => {
    let deep: unknown = "src/app.ts";
    for (let depth = 0; depth < 12; depth += 1) deep = { next: deep };
    expect(callNamesDeniedPath(deep)).toBe(true);
    expect(agentTranscriptPayload("call", { toolName: "Mcp", input: deep })).toEqual({ toolName: "Mcp", input: WITHHELD_INPUT });
    expect(callNamesDeniedPath({ files: Array.from({ length: 1_000 }, (_, index) => `src/file-${index}.ts`) })).toBe(true);
    expect(callNamesDeniedPath({ files: Array.from({ length: 100 }, (_, index) => `src/file-${index}.ts`), sizes: Array.from({ length: 1_000 }, () => 1) }))
      .toBe(false);
  });

  it("withholds every tool output and only the inputs that name a path outside the boundary", () => {
    expect(agentTranscriptPayload("result", { toolName: "Bash", output: "API_KEY=x", failed: false }))
      .toEqual({ toolName: "Bash", output: WITHHELD_OUTPUT, failed: false });
    expect(agentTranscriptPayload("call", { toolName: "Bash", input: { command: "cat .env" } }))
      .toEqual({ toolName: "Bash", input: WITHHELD_INPUT });
    const read = { toolName: "Read", input: { file_path: "/repo/src/a.ts" } };
    expect(agentTranscriptPayload("call", read)).toBe(read);
    const prompt = { text: "please read .env and tell me the key" };
    expect(agentTranscriptPayload("user-prompt", prompt)).toBe(prompt);
    expect(agentTranscriptPayload("result", "not an object")).toBe("not an object");
  });

  it("passes the conversation and Rerail's own bookkeeping whole, and withholds a kind it does not know", () => {
    for (const [kind, payload] of [
      ["session-start", { kind: "session-start", session: { sessionId: "s", host: "codex", cwd: "/repo" } }],
      ["assistant-message", { messageId: "m", text: "I will read .env next" }],
      ["turn-end", { text: "done" }],
      ["steering-delivered", { ruleIds: ["no-console-log"] }],
      ["code-rules-pending", { boundary: "edit" }],
      ["events-dropped", { id: "d", count: 3 }],
    ] as const) expect(agentTranscriptPayload(kind, payload), kind).toBe(payload);
    expect(agentTranscriptPayload("file-snapshot", { path: "src/a.ts", content: "API_KEY=x" })).toEqual(WITHHELD_PAYLOAD);
    expect(agentTranscriptPayload("constructor", "API_KEY=x")).toEqual(WITHHELD_PAYLOAD);
    // Rerail's records of what Git's ignore rules left unchecked quote hidden ignore files: never an agent's to read.
    expect(agentTranscriptPayload("ignored-edit", { path: "scratch/a.ts", source: ".git/info/exclude", line: 7, pattern: "scratch/" }))
      .toEqual(WITHHELD_PAYLOAD);
    expect(agentTranscriptPayload("ignore-rules-changed", { path: ".git/info/exclude", via: "observed" })).toEqual(WITHHELD_PAYLOAD);
  });

  it("withholds a call that names a file linked to a secret in the session's checkout", context => {
    const root = realpathSync(mkdtempSync(join(tmpdir(), "rerail-transcript-boundary-")));
    roots.push(root);
    writeFileSync(join(root, ".env"), "API_KEY=x\n");
    mkdirSync(join(root, "docs"));
    writeFileSync(join(root, "docs", "guide.md"), "# Guide\n");
    symlinkOrSkip(context, "../.env", join(root, "docs", "a.md"));
    const linked = { toolName: "Bash", input: { command: "cat docs/a.md" } };
    expect(agentTranscriptPayload("call", linked)).toBe(linked);
    expect(agentTranscriptPayload("call", linked, root)).toEqual({ toolName: "Bash", input: WITHHELD_INPUT });
    expect(callNamesDeniedPath({ file_path: join(root, "docs", "a.md") }, root)).toBe(true);
    expect(callNamesSecretPath({ command: "cat ./docs/a.md" }, root)).toBe(true);
    // A link at the checkout's root is resolved when it is named like a file, and a bare word is judged by its name.
    symlinkOrSkip(context, ".env", join(root, "notes.txt"));
    expect(callNamesSecretPath({ command: "cat notes.txt" }, root)).toBe(true);
    expect(callNamesSecretPath({ command: "cat token" }, root)).toBe(true);
    const plain = { toolName: "Read", input: { file_path: join(root, "docs", "guide.md") } };
    expect(agentTranscriptPayload("call", plain, root)).toBe(plain);
    expect(callNamesDeniedPath({ command: "cat docs/missing.md" }, root)).toBe(false);
  });

  it("passes a call that names a link into a hidden folder that is not secret-like, as pnpm's node_modules are", () => {
    const root = realpathSync(mkdtempSync(join(tmpdir(), "rerail-transcript-pnpm-")));
    roots.push(root);
    const store = join(root, "node_modules", ".pnpm", "left-pad@1.3.0", "node_modules", "left-pad");
    mkdirSync(store, { recursive: true });
    writeFileSync(join(store, "index.js"), "module.exports = pad;\n");
    linkFolder(join(".pnpm", "left-pad@1.3.0", "node_modules", "left-pad"), join(root, "node_modules", "left-pad"));
    const read = { toolName: "Bash", input: { command: "cat node_modules/left-pad/index.js" } };
    expect(agentTranscriptPayload("call", read, root)).toBe(read);
    // The hidden path itself, named, is still outside.
    expect(callNamesDeniedPath({ command: "cat node_modules/.pnpm/left-pad@1.3.0/node_modules/left-pad/index.js" }, root)).toBe(true);
  });

  it("withholds a call that names a link to a secret in the temp folder, the home folder or another folder outside the checkout", () => {
    const { base, repo } = linksToSecretFolder();
    recordLookups();
    for (const link of [join(tmpdir(), "cfg"), join(homedir(), "cfg"), join(base, "out", "cfg")]) {
      const named = join(link, "config");
      // By absolute path, by `..` path, as a `file:` URL or another URL with no host, which names a path here, and as
      // the word a `:` joins to a URL's host.
      for (const input of [
        { command: `cat ${named}` },
        { file_path: named },
        { command: `cat ${relative(repo, named)}` },
        { command: `curl file://${urlPath(named)}` },
        { command: `DOCKER_HOST=unix://${urlPath(named)} docker ps` },
        { command: `git clone git+file://${urlPath(named)}` },
        { command: `sqlite3 sqlite:///${urlPath(named)}` },
        { command: `curl https://192.0.2.1:${relative(repo, named)}` },
      ]) {
        expect(callNamesSecretPath(input, repo), JSON.stringify(input)).toBe(true);
        expect(agentTranscriptPayload("call", { toolName: "Bash", input }, repo), JSON.stringify(input))
          .toEqual({ toolName: "Bash", input: WITHHELD_INPUT });
      }
    }
  });

  it.skipIf(process.platform === "win32")("looks up a local path that starts with `//` after the `:` of a word or a path, as in a `PATH` list", () => {
    const { base, repo } = linksToSecretFolder();
    const lookups = recordLookups();
    // On Linux and macOS `//<base>/out/cfg/config` is the local path `<base>/out/cfg/config`; on Windows it is a share.
    const named = `/${join(base, "out", "cfg", "config")}`;
    for (const command of [
      `PATH=/usr/bin:${named}`, `LD_LIBRARY_PATH=/opt/app/lib:${named}`, `NODE_PATH=/usr/lib/node_modules:${named}`,
      `PATH=/srv/http:${named}`, `cat ~/notes.md:${named}`, `PATH=https://192.0.2.1/bin:${named}`,
      // After a single word, and after a path that ends in a web scheme's name.
      `PATH=bin:${named}`, `LD_LIBRARY_PATH=lib:${named}`, `PYTHONPATH=src:${named}`, `CLASSPATH=classes:${named}`,
      `MANPATH=man:${named}`, `PATH=//opt/https:${named}`, `PATH=8080/http:${named}`, `PATH=lib.d/https:${named}`,
      `ls a.b/https:${named}`,
    ]) {
      expect(callNamesSecretPath({ command }, repo), command).toBe(true);
      expect(agentTranscriptPayload("call", { toolName: "Bash", input: { command } }, repo), command)
        .toEqual({ toolName: "Bash", input: WITHHELD_INPUT });
    }
    // A `/net` path in such a list is looked up as well, as on `main`: only a URL's host and path under `/net` is not.
    // `~ftp` is the home folder of the user `ftp`, `.https` a hidden folder, `libhttp` a folder whose name ends in one,
    // and a `/net` path with one slash after a scheme's name is no URL's host and path, which starts with `//`.
    const netLists = [
      "PATH=/usr/bin://net/192.0.2.1/bin", "PATH=bin://net/192.0.2.1/bin", "PATH=//opt/https://net/192.0.2.1/bin",
      "PATH=~ftp://net/192.0.2.1/bin", "PATH=~http://net/192.0.2.1/bin", "PATH=.https://net/192.0.2.1/bin",
      "PATH=libhttp://net/192.0.2.1/bin", "PATH=https:/net/192.0.2.1/bin",
    ];
    for (const command of netLists) expect(callNamesSecretPath({ command }, repo), command).toBe(false);
    expect(lookups.filter(reachesAnotherMachine)).toEqual(netLists.map(() => "/net/192.0.2.1/bin"));
  });

  it("judges a link elsewhere in the checkout when the session runs in a subfolder of it", () => {
    const { repo, app } = linksToSecretFolder();
    for (const input of [{ command: "cat ../../docs/cfg/config" }, { command: `cat ${join(repo, "docs", "cfg", "config")}` }]) {
      expect(callNamesSecretPath(input, app), JSON.stringify(input)).toBe(true);
    }
  });

  it("starts no lookup that reaches another machine for a URL's host and path, judges it by name, and looks up every other path-shaped word", () => {
    const base = checkout("rerail-transcript-lookups-");
    const root = join(base, "repo");
    mkdirSync(root);
    const lookups = recordLookups();
    for (const input of [...URL_INPUTS, ...AUTOMOUNT_URL_INPUTS]) {
      expect(callNamesSecretPath(input, root), JSON.stringify(input)).toBe(false);
      // A coding agent reading the session's events has each stored call judged in the same checkout.
      const call = { toolName: "Bash", input };
      expect(agentTranscriptPayload("call", call, root), JSON.stringify(input)).toBe(call);
    }
    expect(callNamesDeniedPath({ url: "https://192.0.2.1/app/.env" }, root)).toBe(true);
    expect(callNamesSecretPath({ url: "https://192.0.2.1/config/secrets.json" }, root)).toBe(true);
    expect(lookups.filter(reachesAnotherMachine)).toEqual([]);
    // Every other path-shaped word is looked up as before: in the checkout, out of it, a `\\server\share` path, also
    // after a `:` and a space, a `file:` URL's host and path, a path a `:` joins to a URL, as in a list, and a path
    // after a URL with no host.
    const share = "\\\\198.51.100.1\\share\\notes.txt";
    const mapped = "\\\\198.51.100.2\\share\\notes.txt";
    const filed = "//198.51.100.3/share/notes.txt";
    const listed = join(base, "bin", "tool.sh");
    const socket = join(base, "run", "app.sock");
    for (const command of [
      "cat docs/guide.md", `cat ${join(base, "notes.txt")}`, `type ${share}`, `net use Z: ${mapped}`, `curl file:${filed}`,
      `PATH=https://192.0.2.1/bin:${listed}`, "curl https://192.0.2.1:docs/joined.md", `DOCKER_HOST=unix://${urlPath(socket)} docker ps`,
    ]) {
      expect(callNamesSecretPath({ command }, root), command).toBe(false);
    }
    expect(lookups).toEqual(expect.arrayContaining([
      join(root, "docs", "guide.md"), join(base, "notes.txt"), resolve(root, share), resolve(root, mapped), resolve(root, filed),
      listed, join(root, "docs", "joined.md"), socket,
    ]));
  });

  it("spends none of a call's lookups on a URL that would reach another machine, so a link named after many is still followed", () => {
    const { repo } = linksToSecretFolder();
    recordLookups();
    // On Windows every such URL's host and path name a share; on Linux and macOS these land under `/net`.
    const urls = Array.from({ length: 80 }, (_, index) => `https://net/192.0.2.1/v${index}`);
    expect(callNamesSecretPath({ content: [...urls, "cat docs/cfg/config"].join("\n") }, repo)).toBe(true);
  });

  it.runIf(process.platform === "win32")("looks up an escaped drive path as its single-backslash spelling: never as a share, never on another drive", () => {
    const { base, repo } = linksToSecretFolder();
    const lookups = recordLookups();
    const named = join(base, "out", "cfg", "config");
    const escaped = named.replaceAll("\\", "\\\\");
    // As JSON, C#, Python or a quoted shell string escape it (`"C:\\Users\\me\\a.txt"`), with `//` after the drive, in
    // brackets, and after the escaped long-path prefix `\\?\`.
    for (const input of [
      { content: JSON.stringify({ path: named }) }, { command: `cat ${named.replace(":\\", "://").replaceAll("\\", "/")}` },
      { content: `[${escaped}](x)` }, { content: JSON.stringify(`\\\\?\\${named}`) },
    ]) {
      expect(callNamesSecretPath(input, repo), JSON.stringify(input)).toBe(true);
      expect(agentTranscriptPayload("call", { toolName: "Write", input }, repo), JSON.stringify(input))
        .toEqual({ toolName: "Write", input: WITHHELD_INPUT });
    }
    expect(lookups.filter(reachesAnotherMachine)).toEqual([]);
    // Each escaped spelling is looked up where `main` looks up the single-backslash one: after a Markdown mark, and on a
    // drive other than the checkout's, whose path `main` looks up on the checkout's drive.
    const lookedUp = (content: string): string[] => {
      lookups.splice(0);
      callNamesSecretPath({ content }, repo);
      return [...lookups];
    };
    const other = /^q/iu.test(repo) ? "R" : "Q";
    expect(lookedUp(`${other}:\\data\\a.txt`)).toEqual([repo, resolve(repo, "\\data\\a.txt")]);
    for (const [spelled, single] of [
      [`**${escaped}**`, `**${named}**`], [`${other}:\\\\data\\\\a.txt`, `${other}:\\data\\a.txt`],
      [`${other}://data/a.txt`, `${other}:\\data\\a.txt`],
    ] as const) {
      expect(lookedUp(spelled), spelled).toEqual(lookedUp(single));
    }
    // A letter is read as a drive at a word's start, after a space, a quote, an equals sign or a `file:` URL's `///`:
    // each spelling is looked up as the single-backslash one, never as a share and never skipped.
    for (const before of ["", "type ", "\"", "path=", "file:///"]) {
      for (const [spelled, single] of [
        ["C:\\\\x\\\\a.txt", "C:\\x\\a.txt"], ["D://x/a.txt", "D:\\x\\a.txt"], ["C:\\\\Users\\\\me", "C:\\Users\\me"],
        ["C://Users/me", "C:\\Users\\me"],
      ] as const) {
        expect(lookedUp(`${before}${spelled}`), `${before}${spelled}`).toEqual(lookedUp(`${before}${single}`));
      }
    }
  });

  it("holds every ASCII character a word can hold, so the table of spellings misses no kind of character", () => {
    const printable = Array.from({ length: 95 }, (_, index) => String.fromCharCode(32 + index));
    const kinds = new Set<string>(WORD_CHARACTERS);
    expect(printable.filter(character => !MAIN_TOKEN_BREAK.test(character) && !/[a-z\d]/iu.test(character) && !kinds.has(character)))
      .toEqual([]);
    expect(WORD_CHARACTERS.filter(character => MAIN_TOKEN_BREAK.test(character))).toEqual([]);
  });

  it("gives every spelling of a word, a `:` and separators `main`'s lookups, but for the changes the description names, and reaches no machine `main` does not", () => {
    const root = checkout("rerail-transcript-spellings-");
    const lookups = recordLookupsOnly(root);
    const lookedUp = (text: string): string[] => {
      lookups.splice(0);
      callNamesSecretPath({ command: text }, root);
      return lookups.filter(path => path !== root);
    };
    const changeOf = process.platform === "win32" ? windowsChange : posixChange;
    // Three verdicts beside each spelling: `main`'s, this check's, and the one the description promises.
    const tally = new Tally();
    for (const spelling of spellings()) tally.add(spelling.before, verdictsOf(spelling, changeOf(spelling), root, lookedUp));
    expect({ broken: tally.broken, byLastTwo: tally.byLastTwo, first: tally.first }).toEqual({ broken: 0, byLastTwo: {}, first: {} });
    // Each change the description names holds the spellings it names; every other spelling is looked up as on `main`.
    expect(tally.changes).toEqual(process.platform === "win32"
      ? { [AS_ON_MAIN]: 218_514, [DRIVE_PATH]: 29_724, [URL_HOST]: 43_482, [NET_URL]: 0 }
      : { [AS_ON_MAIN]: 287_097, [DRIVE_PATH]: 0, [URL_HOST]: 0, [NET_URL]: 4_623 });
    // 291,720 spellings take 2 to 5 s; the bound leaves room on a loaded machine.
  }, 30_000);

  it.runIf(process.platform === "win32")("looks up a device path or a loopback share right after a `:` as `main` does, and skips only a share on another machine there", () => {
    const { base, repo } = linksToSecretFolder();
    const device = onVolume(join(base, "out", "cfg", "config"));
    const lookups = recordLookups();
    // A device path names a local file with no drive colon, so a link reached through one is followed after a `:` too.
    for (const spelled of [device, device.replaceAll("\\", "/"), device.replace("?", ".")]) {
      const input = { command: `ls path:${spelled}` };
      expect(callNamesSecretPath(input, repo), spelled).toBe(true);
      expect(agentTranscriptPayload("call", { toolName: "Bash", input }, repo), spelled)
        .toEqual({ toolName: "Bash", input: WITHHELD_INPUT });
    }
    const lookedUp = (content: string): string[] => {
      lookups.splice(0);
      callNamesSecretPath({ content }, repo);
      return [...lookups];
    };
    // `main` looks up each of these words wherever it stands: the long-path prefix before a drive (`\\?\C:\x`, cut at
    // its `:`), the `?` and `.` hosts with either separator, a share on the loopback host, and the network redirector
    // with no host after it, which names no machine. So does this check, right after a scheme, a word or a drive letter
    // and its `:`, and after a space.
    const here = [
      "\\\\?\\C", "//?/GLOBALROOT/Device/x", "\\\\.\\pipe\\x", "//./net/192.0.2.1/n", "\\/?\\x\\y", "\\\\localhost\\c$\\x",
      "//127.0.0.1/c$/x", "//127.1.2.3/c$/x", "\\\\?\\UNC", "//./UNC/", "\\\\?\\GLOBALROOT\\Device\\Mup",
    ];
    const texts = here.flatMap(word => ["https:", "path:", "C:", "x "].map(before => [`${before}${word}`, word] as const));
    expect(Object.fromEntries(texts.map(([text]) => [text, lookedUp(text)])))
      .toEqual(Object.fromEntries(texts.map(([text, word]) => [text, [repo, resolve(repo, word)]])));
    // A word that names another machine with a URL's `//` is skipped right after a `:`, and looked up as on `main` after
    // a space: a share, one on a name that only starts like the loopback host's, the long-path prefix before a share,
    // and a device path that goes on to the network redirector, also where its `.` and `..` names or a name's trailing
    // period hide it.
    const remote = [
      "//192.0.2.1/s/x", "//localhost.example/s/x", "//127.0.0.256/s/x", "//127.0.0.1./s/x",
      "//?/UNC/192.0.2.1/s/x", "//./UNC/192.0.2.1/s/x", "//?/GLOBALROOT/Device/Mup/192.0.2.1/s/x", "//?/GLOBALROOT/Global??/UNC/192.0.2.1/s/x",
      "//./../UNC/192.0.2.1/s/x", "//././UNC/192.0.2.1/s/x", "//./x/../../UNC/192.0.2.1/s/x",
      "//?/GLOBALROOT/Device/x/../Mup/192.0.2.1/s/x", "//./UNC./192.0.2.1/s/x",
    ];
    expect(Object.fromEntries(remote.map(word => [word, lookedUp(`https:${word}`)])))
      .toEqual(Object.fromEntries(remote.map(word => [word, []])));
    // The same word written with backslashes is no URL's host and path, so it is looked up as on `main` right after a
    // `:` too, a scheme's or a word's.
    const backslashed = remote.map(word => word.replaceAll("/", "\\"));
    const elsewhere = [
      ...remote.map(word => [`x ${word}`, word] as const),
      ...backslashed.flatMap(word => ["https:", "path:", "x "].map(before => [`${before}${word}`, word] as const)),
    ];
    expect(Object.fromEntries(elsewhere.map(([text]) => [text, lookedUp(text)])))
      .toEqual(Object.fromEntries(elsewhere.map(([text, word]) => [text, [repo, resolve(repo, word)]])));
  });

  it.runIf(process.platform === "win32").each(SHARES_AFTER_A_COLON)("looks up another machine's share after a `:` in $form as `main` does, and follows a link there to a secret", ({ command, share }) => {
    const root = checkout("rerail-transcript-share-after-colon-");
    // A stand-in answers the share as a link to a secret folder on that host; no lookup reaches a host.
    const lookups = recordLookups(new Map([[share, "\\\\192.0.2.1\\home\\.aws"]]));
    expect(callNamesSecretPath({ command }, root), command).toBe(true);
    expect(agentTranscriptPayload("call", { toolName: "Bash", input: { command } }, root), command)
      .toEqual({ toolName: "Bash", input: WITHHELD_INPUT });
    // One lookup of the share for each of the two judgments, as on `main`, and none of another machine's path.
    expect(lookups.filter(reachesAnotherMachine)).toEqual([share, share]);
  });

  it.runIf(process.platform === "win32")("looks up a share on the loopback host right after a `:` however its address is written, as `main` does", () => {
    const root = checkout("rerail-transcript-loopback-spellings-");
    const lookups = recordLookups();
    const lookedUp = (content: string): string[] => {
      lookups.splice(0);
      callNamesSecretPath({ content }, root);
      return [...lookups];
    };
    // The address parser reads `127.1`, `2130706433`, `0x7f000001` and `127.0.0.01` as 127.0.0.1, with no name lookup;
    // a device path through the network redirector to the loopback host stays on this machine too.
    const here = [
      "//127.1/c$/x", "\\\\2130706433\\c$\\x", "//0x7f000001/c$/x", "\\\\127.0.0.01\\c$\\x", "//LOCALHOST/c$/x",
      "\\\\?\\UNC\\localhost\\c$\\x", "//./UNC/127.1/c$/x", "\\\\?\\GLOBALROOT\\Device\\Mup\\127.0.0.1\\c$\\x",
    ];
    const texts = here.flatMap(word => ["https:", "path:", "x "].map(before => [`${before}${word}`, word] as const));
    expect(Object.fromEntries(texts.map(([text]) => [text, lookedUp(text)])))
      .toEqual(Object.fromEntries(texts.map(([text, word]) => [text, [root, resolve(root, word)]])));
  });

  it.skipIf(process.platform === "win32")("looks up a URL's host and path under `/net` on the loopback host, which stays on this machine", () => {
    const root = checkout("rerail-transcript-loopback-");
    const lookups = recordLookups();
    // The address parser reads `127.1`, `2130706433`, `0x7f000001` and `127.0.0.01` as 127.0.0.1, with no name lookup.
    const here = ["localhost", "LOCALHOST", "127.0.0.1", "127.1.2.3", "127.1", "2130706433", "0x7f000001", "127.0.0.01"];
    // A host with any other character is read as a name, which may name another machine: `127.0.0.1.` too, which the
    // URL parser would read as 127.0.0.1.
    for (const host of [...here, "192.0.2.1", "localhost.example", "127.0.0.256", "127.0.0.1."]) {
      expect(callNamesSecretPath({ command: `curl https://net/${host}/api/v1` }, root), host).toBe(false);
    }
    expect(lookups.filter(path => path !== root)).toEqual(here.map(host => `/net/${host}/api/v1`));
  });

  it("looks up a share or a `/net` path on this machine's own name right after a `:`, as `main` does", () => {
    const root = checkout("rerail-transcript-own-name-");
    const lookups = recordLookups();
    // The name the system gives this machine, in either letter case: a lookup there stays on this machine.
    const names = [hostname(), hostname().toUpperCase()];
    const words = process.platform === "win32"
      ? names.flatMap(name => [`//${name}/c$/x`, `\\\\${name}\\c$\\x`])
      : names.map(name => `//net/${name}/api/v1`);
    for (const word of words) {
      lookups.splice(0);
      expect(callNamesSecretPath({ command: `curl https:${word}` }, root), word).toBe(false);
      expect(lookups.filter(path => path !== root), word).toEqual([resolve(root, word)]);
    }
  });

  it.runIf(process.platform === "linux")("looks up a URL's host and path under `/NET`, which on Linux is a folder of its own, not the automount root", () => {
    const root = checkout("rerail-transcript-net-case-");
    const lookups = recordLookups();
    for (const command of ["ls https://NET/192.0.2.1/x", "curl http://Net/192.0.2.1/x"]) {
      expect(callNamesSecretPath({ command }, root), command).toBe(false);
    }
    expect(lookups.filter(path => path !== root)).toEqual(["/NET/192.0.2.1/x", "/Net/192.0.2.1/x"]);
  });

  it("judges only secret-like names as secret, not every hidden path", () => {
    expect(callNamesSecretPath({ command: "cat .env" })).toBe(true);
    expect(callNamesSecretPath({ file_path: "/repo/deploy/gcp-key.json" })).toBe(true);
    expect(callNamesSecretPath({ path: "/repo/.github/workflows/ci.yml" })).toBe(false);
    expect(callNamesDeniedPath({ path: "/repo/.github/workflows/ci.yml" })).toBe(true);
    expect(callNamesSecretPath({ command: "cat src/app.ts" })).toBe(false);
  });
});
