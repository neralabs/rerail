import { hostname } from "node:os";
import { posix, win32 } from "node:path";
import { checkoutPathBoundary, deniedByPathBoundary, sensitiveByName, type CheckoutPathBoundary } from "./path-boundary.js";

/**
 * What a coding agent gets in place of transcript content Rerail keeps out of a model conversation: a tool's output,
 * which may be a file it read, a tool call that names a path outside the data boundary
 * ({@link deniedByPathBoundary}): `cat .env`, a read of `.ssh/id_rsa`, a write to `credentials.json`, or the payload of
 * an event kind this code does not know. People read the transcript whole; this is the agent's view, decided here, in
 * code, not in the agent's instructions.
 */
export interface Withheld {
  withheld: "tool output" | "tool input" | "event payload";
  reason: string;
}

export const WITHHELD_OUTPUT: Readonly<Withheld> = {
  withheld: "tool output",
  reason: "a tool's output may be a file's contents; Rerail does not hand one session's reads to another coding agent",
};

export const WITHHELD_INPUT: Readonly<Withheld> = {
  withheld: "tool input",
  reason: "the call names a hidden or secret-like path, which Rerail keeps out of a model conversation",
};

export const WITHHELD_PAYLOAD: Readonly<Withheld> = {
  withheld: "event payload",
  reason: "Rerail does not know what this kind of event holds, so it does not hand it to a coding agent",
};

/** How deep, and how many strings, a tool input is read: a tool input is small, and a pathological one is not read whole. */
const INPUT_DEPTH = 8;
const INPUT_STRINGS = 512;
/** How many of a tool input's path-shaped tokens are resolved on disk, in input order; the rest by name alone. */
const INPUT_RESOLUTIONS = 64;

/**
 * Shell, prose and brace-glob delimiters around a token that may be a path. The split keeps them, at odd indexes, so
 * the run of delimiters right before a token can be read ({@link lookupOf}).
 */
const TOKEN_BREAK = /([\s"'`|;&<>(),:={}]+)/u;

/**
 * On Windows, a token that names a share as `path.resolve` reads it: two separators of either kind, a host, a separator
 * and a share (`//host/api`, `\\host\api`).
 */
const SHARE = /^[\\/]{2}[^\\/]+[\\/]+[^\\/]/u;

/** The loopback host by name or by an IPv4 address in `127.0.0.0/8`, as the URL parser writes them. */
const LOOPBACK = /^(?:localhost|127(?:\.(?:25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)){3})$/u;

/** A host made of ASCII letters and digits in names joined by dots, which an address parser may read as an address. */
const ADDRESS_SPELLING = /^[a-z\d]+(?:\.[a-z\d]+)*$/iu;

/**
 * Whether `host` is the loopback host. The URL parser reads it as the system's address parser does, with no name
 * lookup: `127.1`, `2130706433`, `0x7f000001` and `127.0.0.01` are `127.0.0.1`, and `LOCALHOST` is `localhost`. A host
 * with any other character, or one the URL parser refuses (`127.0.0.256`), is a name, which names another machine.
 */
const isLoopback = (host: string): boolean =>
  ADDRESS_SPELLING.test(host) && LOOPBACK.test(URL.parse(`http://${host}/`)?.hostname ?? "");

/**
 * This machine's own name as the system gives it (`os.hostname()`), read once. Windows resolves its own name to its
 * own addresses before it asks any name server, so a share there stays on this machine; host names ignore letter case.
 */
const THIS_MACHINE = hostname().toLowerCase();

/**
 * Whether a lookup that reaches `host` leaves this machine: it reaches a host, and neither the loopback host nor this
 * machine by its own name. Any other name of this machine (a full DNS name, an alias) is read as another machine's.
 */
const onAnotherMachine = (host: string | undefined): boolean =>
  host !== undefined && !isLoopback(host) && host.toLowerCase() !== THIS_MACHINE;

/** The Windows device namespace's own folders, each followed by its separators: `GLOBALROOT\`, `Device\`, `Global\`. */
const NAMESPACE_FOLDER = String.raw`(?:GLOBALROOT|Global|GLOBAL\?\?|\?\?|DosDevices|Device)[\\/]+`;

/** A path in the Windows device namespace, host `?` or `.`: `\\?\GLOBALROOT\Device\HarddiskVolume3\x`, `\\.\pipe\x`. */
const DEVICE = /^[\\/]{2}[.?][\\/]/u;

/**
 * The host a Windows device path reaches when the first name past the namespace's own folders is the network
 * redirector, `UNC`, `Mup`, `RdpDr` or a name ending in `Redirector`: `host` in `\\?\UNC\host\share` and
 * `\\?\GLOBALROOT\Device\Mup\host\share`. The redirector with no name after it (`\\?\UNC`) names no machine.
 */
const REDIRECTED_HOST = new RegExp(
  String.raw`^[\\/]{2}[.?][\\/]+(?:${NAMESPACE_FOLDER})*(?:UNC|Mup|RdpDr|\w*Redirector)[\\/]+([^\\/]+)`,
  "iu",
);

/** A share's host: `host` in `\\host\share`. */
const SHARE_HOST = /^[\\/]{2}([^\\/]+)/u;

/**
 * On Windows, the host the lookup of a share-shaped path reaches, read from the path it opens
 * ({@link openedOnWindows}): a share's host, or the host past the network redirector in a device path; none for a
 * device path that stays in this machine's own namespace.
 */
const hostReached = (opened: string): string | undefined =>
  (DEVICE.test(opened) ? REDIRECTED_HOST : SHARE_HOST).exec(opened)?.[1];

/** A name's one trailing period, which Windows drops from a path after `\\.\`: `\\.\UNC.\host` opens `\\.\UNC\host`. */
const TRAILING_PERIOD = /(?<=[^\\.])\.(?=\\|$)/gu;

/**
 * On Windows, the path a lookup of the share-shaped `path` opens: `path.resolve` drops its `.` and `..` names, right
 * after a `\\?\` or `\\.\` prefix too (`//./../UNC/host` is `\\.\UNC\host`), and Windows drops a name's trailing period
 * after `\\.\`. A share or device path is absolute, so the checkout it is resolved in does not change it.
 */
function openedOnWindows(path: string): string {
  const resolved = win32.resolve(path);
  return resolved.startsWith("\\\\.\\") ? resolved.replace(TRAILING_PERIOD, "") : resolved;
}

/**
 * On Windows, whether `path` names another machine's share, read from the path its lookup opens: looking one up makes
 * Windows resolve the host's name and open a connection to it, and the lookup waits about 21 s for a host that does
 * not answer. A device path that stays in this machine's namespace, a share on the loopback host however its address
 * is written (`\\127.1\c$`, `\\?\UNC\localhost\c$`), and one on this machine's own name stay on this machine.
 */
const namesAnotherMachine = (path: string): boolean =>
  SHARE.test(path) && onAnotherMachine(hostReached(openedOnWindows(path)));

/**
 * A Windows drive letter at the end of the token before a lone `:`, read from the token's last two characters: `C` in
 * `"C:\\Users\\me"` written in JSON, in `**C`, in `[C` and in the escaped long-path prefix `\\\\?\\C`. A letter after
 * an ASCII letter or digit is part of a word, and one after a dash or a slash is a switch's: `-P` in PowerShell's
 * `-P:\\host\share` (which also takes an en dash, an em dash or a horizontal bar for the dash) and `/G` in findstr's
 * `/G:\\host\share\words.txt`.
 */
const DRIVE_AT_END = /(?:^|[^-a-z\d/\u2013-\u2015])[a-z]$/iu;

/** A run of path separators, which Windows reads as one. */
const SEPARATOR_RUN = /([\\/])[\\/]+/gu;

/**
 * A scheme whose URL names a share outright, `file` or `smb`, ending the token before a `:` as a whole scheme: with
 * nothing before it but marks, or as the last part of a compound scheme (`git+file`). `profile` and `Makefile` are
 * other words, and their URLs are read as any other scheme's.
 */
const SHARE_SCHEME_AT_END = /(?:^|[^a-z\d.-])(?:file|smb)$/iu;

/**
 * On Windows, whether a share on another machine right after a `:` and the token `before` it may be a URL's host and
 * path: it starts with a URL's `//`, after a scheme (`https://host/x`), a placeholder (`${SCHEME}://`, `{}://`), a
 * digit or `%20` glued to a scheme, or nothing, and the scheme is neither `file` nor `smb`. The text cannot tell such a
 * URL's host and path from a share there. A `file:` or `smb:` URL with a host names a share outright, and a share
 * written with backslashes is no URL's (robocopy's `/LOG:\\host\share\run.txt`, PowerShell's `-Path:\\host\share`):
 * both are looked up.
 */
const urlAfter = (before: string, path: string): boolean =>
  path.startsWith("//") && !SHARE_SCHEME_AT_END.test(before);

/** Whether a drive letter ({@link DRIVE_AT_END}) ends the token `before` a lone `:`, the run of `delimiters`. */
const driveBefore = (delimiters: string, before: string): boolean =>
  delimiters === ":" && DRIVE_AT_END.test(before.slice(-2));

/**
 * On Windows, what `path`, after the run of `delimiters` and the token `before` them, is looked up as: a share on
 * another machine ({@link namesAnotherMachine}) right after a `:` that may be a URL's host and path ({@link urlAfter})
 * is not looked up. A drive letter before a lone `:` comes first: the share is then the rest of a drive path written
 * with doubled separators (`"C:\\Users\\me"` in JSON, `C://Users/me`), looked up as `C:\Users\me` is, its path on the
 * checkout's drive. A device path or a loopback share is looked up wherever it stands, as any other token.
 */
function windowsLookup(delimiters: string, before: string, path: string): string | undefined {
  if (!delimiters.endsWith(":") || !namesAnotherMachine(path)) return path;
  if (driveBefore(delimiters, before)) return path.replace(SEPARATOR_RUN, "$1");
  return urlAfter(before, path) ? undefined : path;
}

/**
 * On Linux and macOS, the host under the automount root `/net`: macOS's default `/net -hosts` map, and autofs on Linux,
 * mount `<host>`'s exports when a path there is looked up, and `/net/localhost`, `/net/127.1` and `/net/<this machine's
 * name>` stay on this machine. macOS's default file system ignores letter case, so `/NET` is the automount root there
 * too; on Linux `/NET` is a folder of its own.
 */
const AUTOMOUNTED_HOST = new RegExp(String.raw`^/net/([^/]+)`, process.platform === "darwin" ? "iu" : "u");

/**
 * On Linux and macOS, a web URL's scheme with only marks before it in its token, or a list's number (`https`, `[https`,
 * `**https`, `#https`, `“https`, `<!--https`, `1.https`). Every character but an ASCII letter, a digit or a slash is a
 * mark here, `-`, `_`, `+` and the letters of other scripts too (`詳細はhttps`), so a list entry spelled that way
 * (`PATH=-https://net/fs/bin`) reads as a URL as well. An ASCII letter, digit or slash before it, or a token that
 * starts with `~` or `.`, leaves the token in doubt: it may be a path in a list (`PATH=bin://net/fs/bin`,
 * `PATH=/srv/http://net/fs/bin`, `PATH=.https://net/fs/bin`, or `PATH=~ftp://net/fs/bin` with the home folder of the
 * user `ftp`), and a path in doubt is looked up.
 */
const CERTAIN_SCHEME = /^(?:\d+\.)?(?![~.])[^a-z\d\\/]*(?:https?|wss?|ftps?)$/iu;

/**
 * Whether a `path` that starts with a URL's `//` lands under the automount root on another machine
 * ({@link onAnotherMachine}) once `path.resolve` normalizes it, as the lookup does: `//net/<host>/x`, and
 * `//a.example/../net/<host>/x`, `//./net/<host>/x` and `///net/<host>/x` too. With one slash (`https:/net/<host>/x`)
 * it is no URL's host and path but a list's next entry, as in `PATH=https:/net/<host>/bin`. Only a path that holds
 * `net` is normalized.
 */
const automounted = (path: string): boolean =>
  path.startsWith("//") && /net/iu.test(path) && onAnotherMachine(AUTOMOUNTED_HOST.exec(posix.normalize(path))?.[1]);

/**
 * On Linux and macOS, what `path`, after the run of `delimiters` and the token `before` them, is looked up as: a `//`
 * path is a local path here, looked up as any other, unless it is certainly a URL's host and path under `/net`.
 */
const posixLookup = (delimiters: string, before: string, path: string): string | undefined =>
  delimiters === ":" && CERTAIN_SCHEME.test(before) && automounted(path) ? undefined : path;

/**
 * Each platform reads a token after a `:` as its paths resolve. On Windows a `//` token is a share, which reaches
 * another machine unless its host is this machine by its spelling or its own name ({@link hostReached}), so a share on
 * another machine written with `//` right after a `:` is skipped, unless a `file:` or `smb:` URL or a drive letter
 * comes before. On Linux and macOS it is a local path, so a token in doubt is looked up, as are the entries of a list
 * such as `PATH=bin://home/me/bin`.
 */
const lookupAfter = process.platform === "win32" ? windowsLookup : posixLookup;

/**
 * What the token at `index` of `parts` (`path`, its brackets dropped) is looked up as on disk: undefined for a URL's
 * host and path that would reach another machine, read from the delimiters right before it and the token before them.
 */
const lookupOf = (parts: readonly string[], index: number, path: string): string | undefined =>
  lookupAfter(parts[index - 1] ?? "", parts[index - 2] ?? "", path);

/** A segment that is no name: `.`, `..` and runs of dots, or a JavaScript spread or prose ellipsis (`...props`). */
const NOT_A_NAME = /^(?:\.+|\.{3}.*)$/u;

/**
 * A token shaped like a path: it has a separator, starts with `.` or `~`, or ends in an extension. Only such a token is
 * resolved on disk; a bare word (a commit message's, a flag's value) is judged by its name, which needs no file system.
 */
const PATH_SHAPED = /[\\/]|^[.~]|\.[^.]+$/u;

/** How a token is judged: by `judge` on its own name and, within a checkout when one is known, by where it leads. */
interface TokenJudge {
  readonly judge: (path: string) => boolean;
  readonly boundary: CheckoutPathBoundary | undefined;
}

/** The name a token is judged by: its segments that are names, joined by `/`; none when it has no letter or no name. */
function nameOf(path: string): string | undefined {
  if (!/[A-Za-z]/u.test(path)) return undefined;
  const segments = path.split(/[\\/]+/u).filter(segment => segment !== "" && !NOT_A_NAME.test(segment));
  return segments.length === 0 ? undefined : segments.join("/");
}

/** Whether a path-shaped token leads, through a link, to a secret-like path; a bare word is not looked up, nor is none. */
function leadsToSecret(path: string | undefined, boundary: CheckoutPathBoundary | undefined): boolean {
  // Where a link leads is judged secret-like or not, never hidden: `node_modules/pkg` leads into `.pnpm/`.
  if (path === undefined || boundary === undefined || !PATH_SHAPED.test(path)) return false;
  const led = boundary.leadsTo(path);
  return led !== undefined && sensitiveByName(led);
}

function textNamesPath(text: string, { judge, boundary }: TokenJudge): boolean {
  const parts = text.split(TOKEN_BREAK);
  for (let index = 0; index < parts.length; index += 2) {
    // A bracket class is read as its letters, so `[.]env` is judged as `.env`.
    const path = (parts[index] ?? "").replace(/[[\]]/gu, "");
    const name = nameOf(path);
    if (name === undefined) continue;
    // A URL's host and path that would reach another machine are judged by name and not looked up: on Windows
    // `//host/api` resolves to the network share `\\host\api`, and the lookup waits for that host, about 21 s for one
    // that never answers. Every other token is looked up, an escaped drive path the split cut at its `:` as its
    // single-separator spelling.
    if (judge(name) || leadsToSecret(lookupOf(parts, index, path), boundary)) return true;
  }
  return false;
}

function namesPath(value: unknown, depth: number, budget: { strings: number }, judge: TokenJudge): boolean {
  if (typeof value !== "string" && (typeof value !== "object" || value === null)) return false;
  // What is past the budget was not read, so it is not known to be clean.
  if (depth > INPUT_DEPTH) return true;
  if (typeof value === "string") {
    budget.strings -= 1;
    return budget.strings < 0 || textNamesPath(value, judge);
  }
  return (Array.isArray(value) ? value : Object.values(value)).some(item => namesPath(item, depth + 1, budget, judge));
}

const callNames = (input: unknown, judge: (path: string) => boolean, root: string | undefined): boolean =>
  namesPath(input, 0, { strings: INPUT_STRINGS }, {
    judge,
    boundary: root === undefined ? undefined : checkoutPathBoundary(root, INPUT_RESOLUTIONS),
  });

/**
 * Whether a tool call's input names a path outside the data boundary, wherever the path sits: a shell command, a
 * `file_path` argument, a glob. Every string in the input is read as tokens, broken at shell and prose delimiters and
 * at braces, so `{.env,.npmrc}` is two; a token is a path when it has a letter in it. A path is judged by its own
 * segments, less those that are no name (`.`, `..`, `...props`), so `~/.ssh/id_rsa`, `./.env`, `[.]env`,
 * `/mnt/secrets/..data/password` and `C:\Users\me\.aws\credentials` are all outside, and `src/.hidden/a.ts` is too.
 * With the checkout the call ran in (`root`), a path is also outside when it links to a secret-like one
 * ({@link sensitiveByName}), so `cat docs/a.md` is outside when `docs/a.md` links to `.env`, while a link into a hidden
 * folder alone (pnpm's `node_modules/pkg` into `.pnpm/`) is not; only a path-shaped token (`docs/a.md`, `.envrc`,
 * `notes.txt`) is resolved, the first `INPUT_RESOLUTIONS` distinct ones, and a bare word by its name alone, as is a
 * URL's host and path where a lookup would reach another machine: on Windows a share on another machine written with
 * `//` right after a `:`, where `//host/api` in `https://host/api` or `${SCHEME}://host/api` names a network share,
 * and on Linux and macOS a web URL under the automount root `/net` (`https://net/host/api`) on another machine. Every
 * other path-shaped token is resolved: a `file:` URL's host and path, a URL's path with no host
 * (`unix:///run/app.sock`), a share or `/net` path on the loopback host or on this machine's own name, on Windows a
 * device path (`\\?\...`) wherever it stands, and a drive path written with doubled separators (`"C:\\Users\\me"` in
 * JSON) as `C:\Users\me` is. On Linux and macOS a `//` path is a local one. On Windows a share on another machine that
 * does not follow a `:`, one written with backslashes after it (`/LOG:\\host\share\run.txt`), and a `file:` or `smb:`
 * URL with a host, are resolved too, as paths the call names. It reads names, not a shell: a path built at run time
 * (`cat $(echo .e)nv`) is not seen. An input deeper or larger than it reads counts as naming one.
 */
export function callNamesDeniedPath(input: unknown, root?: string): boolean {
  return callNames(input, deniedByPathBoundary, root);
}

/**
 * Whether a tool call's input names a secret-like path ({@link sensitiveByName}), read as {@link callNamesDeniedPath}
 * reads it: a hidden path that is not secret-like by name (`.github/workflows/ci.yml`) does not count.
 */
export function callNamesSecretPath(input: unknown, root?: string): boolean {
  return callNames(input, sensitiveByName, root);
}

/**
 * How a coding agent sees each stored event kind: the conversation (the adapter's events: `session-start` through
 * `turn-end`) and Rerail's own bookkeeping (what `SessionStore` records: HUD tracking, steering deliveries, dropped
 * events, pending checks, rule coverage) pass `whole`; a `result` passes without its `output`; a `call` passes without
 * its `input` when that names a path outside the boundary. A kind not listed here is withheld whole, so a kind added
 * later reaches an agent only once it is listed; what Git's ignore rules left unchecked, and changes to them, are
 * withheld, since they quote hidden ignore files. A map, not a record, so a stored kind such as `constructor` finds no
 * prototype member.
 */
const AGENT_EVENT_VIEW: ReadonlyMap<string, "whole" | "without output" | "without denied input"> = new Map([
  ["session-start", "whole"],
  ["session-end", "whole"],
  ["session-lifecycle", "whole"],
  ["user-prompt", "whole"],
  ["assistant-delta", "whole"],
  ["assistant-message", "whole"],
  ["assistant-abort", "whole"],
  ["turn-end", "whole"],
  ["call", "without denied input"],
  ["result", "without output"],
  ["hud-tracking-start", "whole"],
  ["steering-delivered", "whole"],
  ["events-dropped", "whole"],
  ["source-discovery-pending", "whole"],
  ["code-rules-pending", "whole"],
  ["rule-coverage", "whole"],
]);

/**
 * A stored transcript event's payload as a coding agent may see it ({@link AGENT_EVENT_VIEW}); with the checkout the
 * session ran in (`root`), a call's input is judged by where its paths lead too ({@link callNamesDeniedPath}).
 */
export function agentTranscriptPayload(kind: string, payload: unknown, root?: string): unknown {
  const view = AGENT_EVENT_VIEW.get(kind);
  if (view === undefined) return WITHHELD_PAYLOAD;
  if (view === "whole" || typeof payload !== "object" || payload === null || Array.isArray(payload)) return payload;
  if (view === "without output") return "output" in payload ? { ...payload, output: WITHHELD_OUTPUT } : payload;
  return "input" in payload && callNamesDeniedPath(payload.input, root) ? { ...payload, input: WITHHELD_INPUT } : payload;
}
