import { realpathSync } from "node:fs";
import { isAbsolute, relative, resolve, sep } from "node:path";

/** The extensions of a data file, as opposed to source: `secrets.json` holds secrets, `secret-manager.ts` handles them. */
const DATA = "{json,yaml,yml,xml,ini,toml,conf,cfg,properties,txt,csv,env}";

/**
 * File names a checkout commonly keeps secrets under: keys, tokens, passwords, cloud and registry credentials,
 * deployment state. Whatever reads a checkout for a model conversation leaves them out, because what a model is shown
 * can end up in its replies, its provider's logs and other tools' input, and nothing a prompt says stops that: the
 * check is here, in code. Matched against a path's last segment, case-insensitively; `*` stands for any run of
 * characters and `{a,b}` for either word. A hidden name (`.ssh/`, `.git-credentials`) is denied as hidden by
 * {@link deniedByPathBoundary}; the hidden names listed here are those {@link sensitiveByName} must judge secret-like
 * without that rule.
 */
export const SENSITIVE_FILE_NAMES: readonly string[] = [
  ".env",
  ".env.*",
  "*.env",
  ".npmrc",
  ".pypirc",
  ".netrc",
  ".pgpass",
  ".htpasswd",
  "id_rsa*",
  "id_dsa*",
  "id_ecdsa*",
  "id_ed25519*",
  "*.pem",
  "*.key",
  "*.p12",
  "*.pfx",
  "*.jks",
  "*.keystore",
  "*.kdbx",
  "*.ppk",
  "*.p8",
  "*.gpg",
  "*.asc",
  "*.ovpn",
  "*.keytab",
  "credentials",
  "credentials.*",
  // `credentials-prod.json`, `credentials_old.yml`: a data file only, so `credentials-helper.ts` is source still.
  `credentials{-,_}*.${DATA}`,
  "secrets.*",
  "*.secret",
  "*.secrets",
  // `app-secrets.yaml`, `client_secret.json`: a data file only, so `secret-manager.ts` is source still.
  `*secret*.${DATA}`,
  "token",
  `token.${DATA}`,
  "*.token",
  ".git-credentials",
  // Shell and REPL histories (`.bash_history`, `.zsh_history`, `.psql_history`) keep the passwords typed into them.
  ".*_history",
  "*.tfstate",
  "*.tfstate.*",
  "*.tfvars",
  "*.tfvars.json",
  "*.sops.*",
  "service-account*.json",
  "serviceaccount*.json",
  // A cloud key file: `gcp-key.json`, `deploy_key.json`; `keyboard.json` and `keymap.json` are not.
  "*-key.json",
  "*_key.json",
  "kubeconfig",
  "kubeconfig.{json,yaml,yml}",
  "*.kubeconfig",
  "auth.json",
  "wp-config.php",
  "database.yml",
  "appsettings.*.json",
];

/** Folders whose every file is a credential or key: a path through one is secret-like whatever the file is called. */
export const SENSITIVE_FOLDER_NAMES: readonly string[] = [".ssh", ".aws", ".gnupg", ".kube", ".docker", ".gcloud", ".azure"];

/** A {@link SENSITIVE_FILE_NAMES} entry as a regular expression's source: `*` is any run of characters, `{a,b}` either word. */
function globSource(glob: string): string {
  const literal = (text: string): string => text.replace(/[.+?^${}()|[\]\\]/gu, "\\$&");
  return glob.replace(/\{([^{}]*)\}|\*|[^*{]+|\{/gu, (match: string, choices: string | undefined) =>
    choices !== undefined ? `(?:${choices.split(",").map(literal).join("|")})` : match === "*" ? ".*" : literal(match));
}

const SENSITIVE_FILE = new RegExp(`^(?:${SENSITIVE_FILE_NAMES.map(globSource).join("|")})$`, "i");
const SENSITIVE_FOLDERS: ReadonlySet<string> = new Set(SENSITIVE_FOLDER_NAMES);

/**
 * Whether a `/`-separated path is named like one holding secrets: its file ({@link SENSITIVE_FILE_NAMES}) or a folder
 * on the way ({@link SENSITIVE_FOLDER_NAMES}), case-insensitively. Narrower than {@link deniedByPathBoundary}: a
 * hidden file is not secret-like by that alone, so a rule about `.github/workflows` still speaks to an agent while a
 * finding in `secrets.yaml` reaches it by rule and place only.
 */
export function sensitiveByName(path: string): boolean {
  const segments = path.split("/").filter(segment => segment !== "");
  const name = segments.at(-1);
  return segments.some(segment => SENSITIVE_FOLDERS.has(segment.toLowerCase())) || (name !== undefined && SENSITIVE_FILE.test(name));
}

/**
 * Whether a checkout-relative, `/`-separated path is outside the data boundary by its name alone: hidden (a segment
 * starts with `.`) or secret-like ({@link sensitiveByName}). It reads nothing on disk ({@link checkoutPathBoundary}
 * also judges where a link leads); what Git ignores is the caller's to add.
 */
export function deniedByPathBoundary(path: string): boolean {
  return path.split("/").some(segment => segment !== "" && segment.startsWith(".")) || sensitiveByName(path);
}

/** How many distinct paths a {@link checkoutPathBoundary} resolves on disk by default; past them a path is judged by name. */
export const CHECKOUT_PATH_RESOLUTIONS = 256;

/** The path boundary within one checkout: where a path leads on disk, and a judgment by its name and by that. */
export interface CheckoutPathBoundary {
  /**
   * Where `path` leads when a link redirects it: its target's `/`-separated path from the checkout's root, or its
   * absolute path when it lies outside. Undefined when no link redirects it or it does not resolve.
   */
  leadsTo(path: string): string | undefined;
  /** {@link sensitiveByName}, of the path and of where it leads. */
  sensitive(path: string): boolean;
}

function realPath(path: string): string | undefined {
  try {
    return realpathSync.native(path);
  } catch {
    return undefined;
  }
}

/**
 * The path boundary for paths relative to (or absolute within) the checkout at `root`, which also judges where a path
 * leads: a tracked `docs/a.md` linked to `../.env` is judged as `.env` too. A path that does not resolve (it does not
 * exist, or `root` does not) is judged by its name alone, as is any path past the first `resolutions` distinct ones,
 * so a boundary makes at most `resolutions + 1` file system calls. Meant to live for one judgment's worth of paths: a
 * steer, a tool call's input.
 */
export function checkoutPathBoundary(root: string, resolutions = CHECKOUT_PATH_RESOLUTIONS): CheckoutPathBoundary {
  const realRoot = realPath(root);
  const targets = new Map<string, string | undefined>();
  const leadsTo = (path: string): string | undefined => {
    if (realRoot === undefined) return undefined;
    if (targets.has(path)) return targets.get(path);
    if (targets.size >= resolutions) return undefined;
    const resolved = realPath(resolve(root, path));
    let led: string | undefined;
    // A path no link redirects is judged by its own name, already.
    if (resolved !== undefined && resolved !== resolve(realRoot, path)) {
      const inside = relative(realRoot, resolved);
      const outside = inside === ".." || inside.startsWith(`..${sep}`) || isAbsolute(inside);
      led = inside === "" ? undefined : (outside ? resolved : inside).split(sep).join("/");
    }
    targets.set(path, led);
    return led;
  };
  return {
    leadsTo,
    sensitive: path => {
      if (sensitiveByName(path)) return true;
      const led = leadsTo(path);
      return led !== undefined && sensitiveByName(led);
    },
  };
}
