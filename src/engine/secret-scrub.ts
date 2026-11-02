import { createHash } from "node:crypto";

/**
 * Redacts secrets from text Rerail writes to disk: stored session events, session ledgers, steers held for a later
 * hook, audit findings and the daemon log. It is applied where text is written, never where it is matched: rules and
 * steers keep reading the original in memory.
 *
 * Two kinds of rule, run in this order over a string:
 * - Pattern rules for formats a secret is recognizable by on its own (provider token prefixes, PEM private keys,
 *   JWTs, the password of a URL, an `Authorization` credential). Each becomes `[redacted:<kind>:<hash>]`, `<hash>`
 *   the first 6 hex digits of the secret's SHA-256, so two mentions of the same secret stay recognizably equal.
 * - The keyed rule: a value assigned to a secret-like key (`password`, `token`, `api_key`, `client_secret`, ... in
 *   `key = value`, `key: value`, `"key": "value"`, env and YAML forms) of at least {@link KEYED_MIN_LENGTH} characters
 *   and {@link KEYED_MIN_ENTROPY} bits of Shannon entropy per character, unless it is a placeholder (`changeme`,
 *   `xxx`, `${VAR}`, `<...>`, `***`) or a filesystem path (`PWD=/Users/...`). A bare value is also kept when it is
 *   code: a call, a member path such as `process.env.TOKEN`, an interpolation such as `$TOKEN` or `$(cat file)`, or an
 *   identifier made of words (`getAuthTokenFromHeader`, `kubernetes-secret-name`). It may be a low-entropy password,
 *   so it becomes `[redacted:<kind>]` with no hash a guess could be checked against.
 *
 * Never redacted: 40 and 64 hex digit hashes, UUIDs, `sha1-`/`sha256-`/`sha384-`/`sha512-` integrity strings and
 * `data:` URIs. Nothing is redacted for its entropy alone, outside a secret-like key.
 *
 * Cost: every expression is linear in the string: no nested quantifiers, every repetition bounded, each anchored on a
 * literal or a character class. A rule whose match can fail after a long scan (a JWT without its dots, an OpenAI key
 * without a digit) starts only where its token starts, not after a character of its own class, so no stretch of text
 * is scanned again from every prefix inside it; a string costs a fixed number of passes. A string with none of the
 * rules' anchor words skips them all after one pass. A string longer than {@link MAX_SCRUB_CHARS} is not scanned: it
 * is replaced whole by `[redacted:unscanned]`, failing closed. A marker matches no rule, so scrubbing is idempotent.
 */

/** The longest string scanned; a longer one is replaced whole (fail closed). */
export const MAX_SCRUB_CHARS = 8 * 1024 * 1024;
/** The shortest value the keyed rule redacts. */
const KEYED_MIN_LENGTH = 12;
/**
 * The least Shannon entropy, in bits per character, of a value the keyed rule redacts. Calibrated on short values: a
 * random 12-character password scores about 3.3–3.6 (the most 12 characters can score is log2(12) ≈ 3.58), a
 * dictionary password such as `mysecretpassword` 3.45, a repeated filler such as `aaaaaaaaaaaa` 0 and
 * `hunter2hunter2` 2.81. A secret-like key is evidence enough, so the bar only drops fillers and repetition.
 */
const KEYED_MIN_ENTROPY = 3.0;
/** Deeper structures are not walked: the value there is replaced (fail closed). */
const MAX_DEPTH = 256;
const MARKER = "[redacted:";

/** The secret-like key names the keyed rule recognizes, matched at the end of a key (`DB_PASSWORD`, `authToken`). */
const KEY_TERMS = "password|passwd|pwd|secret|token|api[_-]?key|access[_-]?key|private[_-]?key|client[_-]?secret|auth";
const SECRET_KEY_NAME = new RegExp(`(?:${KEY_TERMS})(?:[_-]?key)?$`, "i");

/** One pass over a string decides whether any rule can apply: every rule needs one of these. */
const ANCHORS = new RegExp(
  `AKIA|ASIA|gh[pousr]_|github_pat_|glpat-|xox[abprs]-|_live_|AIza|sk-|npm_|pypi-|PRIVATE KEY|eyJ|://|Bearer|Basic|${KEY_TERMS}`,
  "i",
);

type Replacer = (match: string, ...groups: Array<string | undefined>) => string;

interface PatternRule {
  readonly kind: string;
  readonly pattern: RegExp;
  /** The redacted form of a match; by default the whole match is the secret. */
  readonly replace?: (kind: string) => Replacer;
}

/** The marker for a pattern rule's match: its kind and the first 6 hex digits of the secret's SHA-256. */
function marker(kind: string, secret: string): string {
  return `${MARKER}${kind}:${createHash("sha256").update(secret).digest("hex").slice(0, 6)}]`;
}

const whole = (kind: string): Replacer => match => marker(kind, match);

/** Group 1 is kept, group 2 is the secret, the rest of the match follows it unchanged. */
const afterPrefix = (kind: string): Replacer => (match, prefix = "", secret = "") =>
  `${prefix}${marker(kind, secret)}${match.slice(prefix.length + secret.length)}`;

/** The most text one PEM private key block spans: an 8192-bit RSA key is about 6.5 KB. */
const MAX_PEM_CHARS = 20_000;
const PEM_BEGIN = /-----BEGIN[A-Z0-9 ]{0,40} PRIVATE KEY-----/g;
const PEM_END = /-----END[A-Z0-9 ]{0,40} PRIVATE KEY-----/y;
/** What a PEM block cut off before its END line runs on with: base64, its headers, and escaped line breaks. */
const PEM_BODY = /[A-Za-z0-9+/=\s\\:,.-]{0,20000}/y;

/**
 * Every PEM private key block, through its END line within {@link MAX_PEM_CHARS}, else (cut off) as far as its base64
 * runs. A scan rather than one expression: the next private key END line is found once and reused by every BEGIN
 * before it, and positions only move forward, so many BEGINs without an END cost one pass, not one per BEGIN.
 */
function redactPrivateKeys(text: string): string {
  let redacted = "";
  let copied = 0;
  /** The next private key END line at or after the current block's body, its end, or -1 when there is none. */
  let end = { at: -1, to: -1 };
  let searchFrom = 0;
  PEM_BEGIN.lastIndex = 0;
  for (let begin = PEM_BEGIN.exec(text); begin !== null; begin = PEM_BEGIN.exec(text)) {
    const bodyStart = begin.index + begin[0].length;
    if (end.at < bodyStart && searchFrom !== -1) {
      searchFrom = Math.max(searchFrom, bodyStart);
      end = { at: -1, to: -1 };
      for (let at = text.indexOf("-----END", searchFrom); at !== -1; at = text.indexOf("-----END", at + 1)) {
        PEM_END.lastIndex = at;
        if (PEM_END.test(text)) {
          end = { at, to: PEM_END.lastIndex };
          break;
        }
      }
      searchFrom = end.at === -1 ? -1 : end.at;
    }
    let blockEnd = end.at >= bodyStart && end.at - begin.index <= MAX_PEM_CHARS ? end.to : -1;
    if (blockEnd === -1) {
      PEM_BODY.lastIndex = bodyStart;
      PEM_BODY.test(text);
      blockEnd = PEM_BODY.lastIndex;
    }
    redacted += text.slice(copied, begin.index) + marker("private-key", text.slice(begin.index, blockEnd));
    copied = blockEnd;
    PEM_BEGIN.lastIndex = blockEnd;
  }
  return copied === 0 ? text : redacted + text.slice(copied);
}

const PATTERN_RULES: readonly PatternRule[] = [
  { kind: "aws-access-key", pattern: /\b(?:AKIA|ASIA)[0-9A-Z]{16}\b/g },
  {
    kind: "aws-secret-key",
    pattern: /((?:aws_?secret_?access_?key|aws_?secret_?key|secret_?access_?key)["']?\s{0,8}(?:=>|:=|=|:)\s{0,8}["']?)([A-Za-z0-9/+]{40})(?![A-Za-z0-9/+])/gi,
    replace: afterPrefix,
  },
  { kind: "github-token", pattern: /\b(?:ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{36,255}\b/g },
  { kind: "github-token", pattern: /\bgithub_pat_[A-Za-z0-9_]{22,255}\b/g },
  { kind: "gitlab-token", pattern: /\bglpat-[A-Za-z0-9_-]{20,255}/g },
  { kind: "slack-token", pattern: /\bxox[abprs]-[A-Za-z0-9-]{10,255}/g },
  { kind: "stripe-key", pattern: /\b(?:sk|rk)_live_[A-Za-z0-9]{16,255}\b/g },
  // A token starts where no character of its class precedes it: after `-`, as `\b` allowed, every `AIza` in a run of
  // `AIza-AIza-...` would scan the next 35 characters.
  { kind: "google-api-key", pattern: /(?<![0-9A-Za-z_-])AIza[0-9A-Za-z_-]{35}(?![0-9A-Za-z_-])/g },
  { kind: "anthropic-key", pattern: /\bsk-ant-[A-Za-z0-9_-]{20,255}/g },
  // `sk-proj-`, `sk-svcacct-`, `sk-admin-` and the original `sk-` keys; a digit tells a key from a kebab-case name. The
  // digit look-ahead scans up to 255 characters, so a key starts only where no character of its class precedes it.
  { kind: "openai-key", pattern: /(?<![A-Za-z0-9_-])sk-(?!ant-)(?=[A-Za-z0-9_-]{0,255}\d)[A-Za-z0-9_-]{20,255}/g },
  { kind: "npm-token", pattern: /\bnpm_[A-Za-z0-9]{36}\b/g },
  { kind: "pypi-token", pattern: /\bpypi-[A-Za-z0-9_-]{50,1000}/g },
  // A JWT starts where no base64url character precedes it: with `\b`, every `eyJ` after a `-` in `eyJ-eyJ-...` began
  // a scan of up to 4096 characters for a dot that never came. Each segment stops at the first character outside its
  // class, and a match can start only past the end of the previous start's first segment, so each run of base64url is
  // scanned at most three times (as a header, a payload and a signature).
  { kind: "jwt", pattern: /(?<![A-Za-z0-9_-])eyJ[A-Za-z0-9_-]{8,4096}\.eyJ[A-Za-z0-9_-]{8,8192}\.[A-Za-z0-9_-]{10,4096}/g },
  // `scheme://user:password@host`: only the password. The scheme starts where no scheme character precedes it, so a
  // run such as `a.a.a.a://` is one start, not one per letter.
  {
    kind: "url-password",
    pattern: /((?<![A-Za-z0-9+.-])[a-z][a-z0-9+.-]{1,20}:\/\/[^\s:/?#@"'<>]{1,256}:)([^\s/?#@"'<>]{1,256})@/gi,
    replace: kind => (match, prefix = "", secret = "") => (placeholder(secret) ? match : `${prefix}${marker(kind, secret)}@`),
  },
  {
    kind: "authorization",
    pattern: /(\b(?:Bearer|Basic)\s{1,8})([A-Za-z0-9._~+/-]{16,4096}={0,2})/g,
    replace: kind => (match, prefix = "", secret = "") => (credential(prefix, secret) ? `${prefix}${marker(kind, secret)}` : match),
  },
];

/**
 * A key ending in a secret-like term, its separator (with the key's closing quote) and its value: double-quoted,
 * single-quoted or bare up to white space or punctuation that ends a value.
 */
const KEYED = new RegExp(
  `(?<![A-Za-z0-9_-])([A-Za-z0-9_-]{0,64}?(?:${KEY_TERMS})(?:[_-]?key)?)`
    + `(["']?[ \\t]{0,8}(?:=>|:=|=|:)[ \\t]{0,8})`
    + `(?:"([^"\\\\\\r\\n]{1,512})"|'([^'\\\\\\r\\n]{1,512})'|([^\\s"'\`,;)\\]}]{1,512}))`,
  "gi",
);

/** True for a value that stands in for a secret rather than being one. */
function placeholder(value: string): boolean {
  const lower = value.toLowerCase();
  return /^(?:x{3,}|\*{3,}|\.{3,}|-{3,}|_{3,})$/.test(lower)
    || /change[-_]?me/.test(lower)
    || /^\$\{[^}]*\}$/.test(value)
    || /^\$[A-Za-z_][A-Za-z0-9_]*$/.test(value)
    || /^<[^>]*>$/.test(value)
    || /^\{\{[^}]*\}\}$/.test(value)
    || /^%\([^)]*\)s$/.test(value)
    || value.includes(MARKER);
}

/** True for values the scrubber never redacts: hashes, UUIDs, integrity strings and `data:` URIs. */
function exempt(value: string): boolean {
  return /^(?:[0-9a-f]{40}|[0-9a-f]{64})$/i.test(value)
    || /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(value)
    || /^sha(?:1|256|384|512)-[A-Za-z0-9+/=]+$/.test(value)
    || /^data:/i.test(value);
}

/**
 * True for a filesystem path: absolute with two or more segments (`/Users/jeff/project`, as `PWD` and `OLDPWD` hold),
 * home- or dot-relative (`~/.config`, `./certs/dev`), or under a Windows drive. Its segments hold no `+`, `=`, `$` or
 * other symbol a generated secret is made of.
 */
function filesystemPath(value: string): boolean {
  return /^(?:\/[\w.@-]+){2,}\/?$/.test(value)
    || /^(?:~|\.{1,2})(?:\/[\w.@-]*)+$/.test(value)
    || /^[A-Za-z]:(?:[\\/][\w .@-]*)+$/.test(value);
}

/** A vowel or `y`: every word of an identifier holds one, unless it is a short acronym. */
const VOWEL = /[aeiouy]/i;

/**
 * True for an identifier made of words, as code names things: `getAuthTokenFromHeader`, `authMiddlewareFactory`,
 * `kubernetes-secret-name`, `MY_SECRET_NAME`, `oauth2Client`. Split at `_`, `-`, `.` and camelCase humps, it has two or
 * more words of letters, three letters long on average, one of them three or more letters with a vowel. Every word has
 * two or more letters, holds a vowel and no run of five other letters, or is an acronym of up to five capitals (`XML`,
 * `HTTP`), one at most beside lower-case words; a longer run of capitals is a word only in a part with no lower-case
 * letter (`SECRET` in `MY_SECRET_NAME`). A digit run ends a word, so no lower-case letter follows a digit. A generated
 * secret splits into one-letter humps, vowel-less fragments, runs of capitals and digits inside a lower-case run
 * (`Zk9xQ2mP7vLw8s` gives `Zk`, `9`, `x`, ...), so it is not one: about 1 in 1,000 random 16-character alphanumeric
 * strings passes. One word on its own (`mysecretpassword`) is judged as any other value.
 */
function wordIdentifier(value: string): boolean {
  if (!/^[A-Za-z_][A-Za-z0-9_]*(?:[.-][A-Za-z0-9_]+)*$/.test(value) || /\d[a-z]/.test(value)) return false;
  let words = 0;
  let letters = 0;
  let camelCaseAcronyms = 0;
  let voweledWord = false;
  for (const part of value.split(/[._-]+/)) {
    const camelCase = /[a-z]/.test(part);
    for (const word of part.match(/[A-Z]+(?![a-z])|[A-Z]?[a-z]+/g) ?? []) {
      const acronym = /^[A-Z]+$/.test(word);
      const vowel = VOWEL.test(word);
      if (word.length < 2) return false;
      if (acronym ? word.length > 5 && (camelCase || !vowel) : !vowel || /[^aeiouy]{5}/i.test(word)) return false;
      if (acronym && camelCase && ++camelCaseAcronyms > 1) return false;
      words += 1;
      letters += word.length;
      voweledWord ||= vowel && word.length >= 3;
    }
  }
  return words >= 2 && letters >= 3 * words && voweledWord;
}

/**
 * True for a bare value that is code, not a literal: a call, an index, a logical operator, a member path, an
 * interpolation (`$TOKEN`, `${TOKEN}`, `$(cat file)`: a `$` that starts the value; one inside a generated password
 * such as `Zk9$xQ2mP7vLw8s` is part of it) or an identifier made of words.
 */
function code(value: string): boolean {
  return /^\$|[([{<]|\|\||&&|\?\?|\?\./.test(value)
    || /^[A-Za-z_]\w*(?:\.[A-Za-z_]\w*)+$/.test(value)
    || wordIdentifier(value);
}

/** True for a `Bearer` token or `Basic` credential, not prose such as "Bearer authentication". */
function credential(prefix: string, token: string): boolean {
  if (placeholder(token) || exempt(token)) return false;
  if (/^basic/i.test(prefix)) {
    const decoded = Buffer.from(token, "base64").toString("utf8");
    return /^[\x20-\x7e]+:[\x20-\x7e]*$/.test(decoded);
  }
  return /\d/.test(token) && /[A-Za-z]/.test(token);
}

/** Bits of Shannon entropy per character. */
function shannonEntropy(value: string): number {
  const counts = new Map<string, number>();
  for (const character of value) counts.set(character, (counts.get(character) ?? 0) + 1);
  let entropy = 0;
  const length = [...value].length;
  for (const count of counts.values()) {
    const p = count / length;
    entropy -= p * Math.log2(p);
  }
  return entropy;
}

/**
 * The kind a secret-like key names, for its marker: `password`, `token`, `api-key`, `access-key`, `private-key`,
 * `secret` or `auth`.
 */
function keyKind(key: string): string {
  const lower = key.toLowerCase();
  if (/(?:password|passwd|pwd)$/.test(lower)) return "password";
  if (/api[_-]?key$/.test(lower)) return "api-key";
  if (/access[_-]?key$/.test(lower)) return "access-key";
  if (/private[_-]?key$/.test(lower)) return "private-key";
  if (/token(?:[_-]?key)?$/.test(lower)) return "token";
  if (/auth(?:[_-]?key)?$/.test(lower)) return "auth";
  return "secret";
}

/** True for a value assigned to a secret-like key that the keyed rule redacts. */
function keyedSecret(value: string, quoted: boolean): boolean {
  if (value.length < KEYED_MIN_LENGTH || placeholder(value) || exempt(value) || filesystemPath(value)) return false;
  if (!quoted && code(value)) return false;
  return shannonEntropy(value) >= KEYED_MIN_ENTROPY;
}

const keyedReplacer: Replacer = (match, key = "", separator = "", double, single, bare) => {
  const value = double ?? single ?? bare ?? "";
  if (!keyedSecret(value, bare === undefined)) return match;
  const redacted = `${MARKER}${keyKind(key)}]`;
  const quote = double !== undefined ? "\"" : single !== undefined ? "'" : "";
  return `${key}${separator}${quote}${redacted}${quote}`;
};

/** `text` with every secret the rules recognize redacted; the same string when there is none. */
export function scrubSecrets(text: string): string {
  if (text.length > MAX_SCRUB_CHARS) return `${MARKER}unscanned]`;
  if (!ANCHORS.test(text)) return text;
  let scrubbed = redactPrivateKeys(text);
  for (const rule of PATTERN_RULES) {
    rule.pattern.lastIndex = 0;
    scrubbed = scrubbed.replace(rule.pattern, (rule.replace ?? whole)(rule.kind) as (substring: string, ...args: unknown[]) => string);
  }
  KEYED.lastIndex = 0;
  return scrubbed.replace(KEYED, keyedReplacer as (substring: string, ...args: unknown[]) => string);
}

/** True when `text` holds a secret the rules recognize. */
export function containsSecret(text: string): boolean {
  return scrubSecrets(text) !== text;
}

function plainObject(value: object): value is Record<string, unknown> {
  const prototype = Object.getPrototypeOf(value) as unknown;
  return prototype === Object.prototype || prototype === null;
}

/** The children of a structure {@link scrubValue} walks, each with its key; undefined for any other value. */
function children(value: object): Array<readonly [unknown, unknown]> | undefined {
  if (Array.isArray(value)) return value.map((item: unknown, index) => [index, item] as const);
  if (value instanceof Map) return [...(value as Map<unknown, unknown>)];
  if (value instanceof Set) return [...(value as Set<unknown>)].map(item => [undefined, item] as const);
  if (plainObject(value)) return Object.entries(value);
  return undefined;
}

/** A string as stored under `key`: a secret-like key makes it the value of an assignment, as the keyed rule judges one. */
function scrubbedString(key: unknown, text: string): string {
  if (typeof key === "string" && SECRET_KEY_NAME.test(key) && keyedSecret(text, true) && !containsSecret(text)) return `${MARKER}${keyKind(key)}]`;
  return scrubSecrets(text);
}

/** Whether scrubbing changes anything in `value`; `seen` holds the structures already judged, so a cycle ends. */
function changes(value: unknown, key: unknown, depth: number, seen: Set<object>): boolean {
  if (typeof value === "string") return scrubbedString(key, value) !== value;
  if (value === null || typeof value !== "object" || seen.has(value)) return false;
  const entries = children(value);
  if (entries === undefined) return false;
  if (depth >= MAX_DEPTH) return true;
  seen.add(value);
  return entries.some(([childKey, item]) => changes(item, childKey, depth + 1, seen));
}

/** A scrubbed copy of `value`; `copies` maps each structure copied to its copy, so a cycle or a shared one stays so. */
function scrubbedCopy(value: unknown, key: unknown, depth: number, copies: Map<object, unknown>): unknown {
  if (typeof value === "string") return scrubbedString(key, value);
  if (value === null || typeof value !== "object") return value;
  if (copies.has(value)) return copies.get(value);
  const entries = children(value);
  if (entries === undefined) return value;
  if (depth >= MAX_DEPTH) return `${MARKER}too-deep]`;
  const copy = (item: unknown, childKey: unknown): unknown => scrubbedCopy(item, childKey, depth + 1, copies);
  if (Array.isArray(value)) {
    const array: unknown[] = [];
    copies.set(value, array);
    for (const [index, item] of entries) array.push(copy(item, index));
    return array;
  }
  if (value instanceof Map) {
    const map = new Map<unknown, unknown>();
    copies.set(value, map);
    for (const [mapKey, item] of entries) map.set(mapKey, copy(item, mapKey));
    return map;
  }
  if (value instanceof Set) {
    const set = new Set<unknown>();
    copies.set(value, set);
    for (const [, item] of entries) set.add(copy(item, undefined));
    return set;
  }
  const object = Object.create(Object.getPrototypeOf(value) as object | null) as Record<string, unknown>;
  copies.set(value, object);
  // Defined, not assigned: an own `__proto__` key (as `JSON.parse` makes one) stays a key, never a prototype.
  for (const [objectKey, item] of entries) {
    Object.defineProperty(object, objectKey as string, { value: copy(item, objectKey), enumerable: true, writable: true, configurable: true });
  }
  return object;
}

/**
 * `value` with every string in it scrubbed ({@link scrubSecrets}), through arrays, plain objects, maps and sets; a
 * string under a secret-like key is judged as the keyed rule judges an assignment (`{ "apiKey": "..." }`). Returns
 * `value` itself when nothing changes, else a copy that keeps its cycles and shared parts; other objects (dates,
 * buffers, class instances) are kept as they are. A structure nested deeper than {@link MAX_DEPTH} is replaced.
 */
export function scrubValue<T>(value: T): T {
  return changes(value, undefined, 0, new Set()) ? scrubbedCopy(value, undefined, 0, new Map()) as T : value;
}

/**
 * Makes everything written to `stream` go through {@link scrubSecrets}: the daemon's stdout and stderr are
 * `daemon.log`, so a logged error that quotes a secret is written redacted. Each write is scrubbed on its own: Rerail
 * writes a log line in one call, and a secret split across two writes is not seen whole. Output written to the file
 * descriptor directly, as the native code's `eprintln!` and Node's report of an uncaught error do, passes unscrubbed.
 */
export function scrubWrites(stream: NodeJS.WriteStream): void {
  const write = stream.write.bind(stream) as (chunk: unknown, ...rest: unknown[]) => boolean;
  stream.write = ((chunk: unknown, ...rest: unknown[]) => {
    // A string in another encoding (`base64`, `hex`) is not text to scrub.
    const encoding = typeof rest[0] === "string" ? rest[0].toLowerCase() : "utf8";
    if (typeof chunk === "string" && (encoding === "utf8" || encoding === "utf-8")) return write(scrubSecrets(chunk), ...rest);
    if (chunk instanceof Uint8Array) {
      const text = Buffer.from(chunk.buffer, chunk.byteOffset, chunk.byteLength).toString("utf8");
      const scrubbed = scrubSecrets(text);
      return write(scrubbed === text ? chunk : Buffer.from(scrubbed, "utf8"), ...rest);
    }
    return write(chunk, ...rest);
  }) as typeof stream.write;
}
