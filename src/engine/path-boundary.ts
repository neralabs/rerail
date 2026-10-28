/**
 * File names a checkout commonly keeps secrets under: keys, tokens, passwords, cloud and registry credentials,
 * deployment state. Whatever reads a checkout for a model conversation leaves them out, because what a model is shown
 * can end up in its replies, its provider's logs and other tools' input, and nothing a prompt says stops that: the
 * check is here, in code. Matched against a path's last segment, case-insensitively; `*` stands for any run of
 * characters. Hidden names (`.ssh/`, `.aws/`, `.git-credentials`) are left out as hidden, so none is listed here.
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
  "secrets.*",
  "*.secret",
  "*.secrets",
  "*.tfstate",
  "*.tfstate.*",
  "*.tfvars",
  "service-account*.json",
  "auth.json",
  "wp-config.php",
  "database.yml",
  "appsettings.*.json",
];

/** Folders whose every file is a credential or key: a path through one is secret-like whatever the file is called. */
export const SENSITIVE_FOLDER_NAMES: readonly string[] = [".ssh", ".aws", ".gnupg", ".kube", ".docker", ".gcloud", ".azure"];

const SENSITIVE_FILE = new RegExp(
  `^(?:${SENSITIVE_FILE_NAMES.map(name => name.split("*").map(part => part.replace(/[.+?^${}()|[\]\\]/g, "\\$&")).join(".*")).join("|")})$`,
  "i",
);
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
 * starts with `.`) or secret-like ({@link sensitiveByName}). It reads nothing on disk; what Git ignores is the
 * caller's to add.
 */
export function deniedByPathBoundary(path: string): boolean {
  return path.split("/").some(segment => segment !== "" && segment.startsWith(".")) || sensitiveByName(path);
}
