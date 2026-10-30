import { createHash } from "node:crypto";
import { describe, expect, it } from "vitest";
import { containsSecret, MAX_SCRUB_CHARS, scrubSecrets, scrubValue } from "../../src/engine/secret-scrub.js";

const hash = (secret: string): string => createHash("sha256").update(secret).digest("hex").slice(0, 6);
/**
 * A fake secret assembled from parts, so the literal never sits whole in this file: secret scanners (GitHub push
 * protection) refuse a push that holds anything shaped like a real key, fake or not.
 */
const fake = (...parts: string[]): string => parts.join("");
const AWS_ID = fake("AKIA", "Z3MPLE7Q2XK4RT6W");
const AWS_STS_ID = fake("ASIA", "Y7Q2XK4RT6WZ3MPL");
const AWS_SECRET = fake("wJalrXUtnFEMI", "/K7MDENG", "/bPxRfiCYzQ8xLp2Vt9");
const GITHUB_PAT_CLASSIC = fake("ghp_", "1A2b3C4d5E6f7G8h9I0jKlMnOpQrStUvWxYz");
const GITHUB_OAUTH = fake("gho_", "Zy9Xw8Vu7Ts6Rq5Po4Nm3Lk2Ji1Hg0FeDcBa");
const GITHUB_PAT = fake("github_pat_", "11ABCDE2F0aBcDeFgHiJkL_mNoPqRsTuVwXyZ0123456789");
const GITLAB = fake("glpat", "-x7Kq2mZ9vL4pR8tN1wB6");
const SLACK = fake("xoxb", "-2048-4096-Zq8Lm2Vx9Rt");
const STRIPE = fake("sk_live", "_4eC39HqLyjWDarjtT1zdp7dc");
const STRIPE_RESTRICTED = fake("rk_live", "_51Hx2mZ9vL4pR8tN1wB6Kq");
const GOOGLE = fake("AIza", "SyD4x7Kq2mZ9vL4pR8tN1wB6cJ5dS0gYa-_");
const OPENAI_PROJECT = fake("sk-", "proj-Q7f2LmZ9xV4pR8tK1wN6bH3cJ5dS0gYa");
const OPENAI = fake("sk-", "T3Blbk", "FJ9xV4pR8tK1wN6bH3cJ5dS0gYaQ7f2LmZ");
const ANTHROPIC = fake("sk-ant-", "api03-Zq8Lm2Vx9Rt4Kp7Wn1");
const NPM = fake("npm_", "a1B2c3D4e5F6g7H8i9J0k1L2m3N4o5P6q7R8");
const PYPI = fake("pypi-", "AgEIcHlwaS5vcmcCJGE4ZjM2ZTJiLWI1ZDMtNGY3Ny1hMjQxLWQ2YzFmMmE5YjQ0NwACKlsz");

/** Each recognized format: the text around it, and the secret the marker stands for. */
const PATTERN_SECRETS: ReadonlyArray<{ kind: string; text: string; secret: string }> = [
  { kind: "aws-access-key", text: `aws_access_key_id = ${AWS_ID}`, secret: AWS_ID },
  { kind: "aws-access-key", text: `export AWS_ACCESS_KEY_ID=${AWS_STS_ID}`, secret: AWS_STS_ID },
  { kind: "aws-secret-key", text: `aws_secret_access_key = ${AWS_SECRET}`, secret: AWS_SECRET },
  { kind: "github-token", text: `token ${GITHUB_PAT_CLASSIC}`, secret: GITHUB_PAT_CLASSIC },
  { kind: "github-token", text: GITHUB_OAUTH, secret: GITHUB_OAUTH },
  { kind: "github-token", text: `GH_TOKEN=${GITHUB_PAT}`, secret: GITHUB_PAT },
  { kind: "gitlab-token", text: GITLAB, secret: GITLAB },
  { kind: "slack-token", text: `SLACK=${SLACK}`, secret: SLACK },
  { kind: "stripe-key", text: `stripe.api_key = ${STRIPE}`, secret: STRIPE },
  { kind: "stripe-key", text: STRIPE_RESTRICTED, secret: STRIPE_RESTRICTED },
  { kind: "google-api-key", text: `key=${GOOGLE}`, secret: GOOGLE },
  { kind: "openai-key", text: `OPENAI=${OPENAI_PROJECT}`, secret: OPENAI_PROJECT },
  { kind: "openai-key", text: OPENAI, secret: OPENAI },
  { kind: "anthropic-key", text: `x-api-key: ${ANTHROPIC}`, secret: ANTHROPIC },
  { kind: "npm-token", text: `//registry.npmjs.org/:_authToken=${NPM}`, secret: NPM },
  { kind: "pypi-token", text: `password = ${PYPI}`, secret: PYPI },
  {
    kind: "private-key",
    text: "-----BEGIN RSA PRIVATE KEY-----\nProc-Type: 4,ENCRYPTED\nMIIEpAIBAAKCAQEA7x9Kq2mZ\nvL4pR8tN1wB6==\n-----END RSA PRIVATE KEY-----\nnext line",
    secret: "-----BEGIN RSA PRIVATE KEY-----\nProc-Type: 4,ENCRYPTED\nMIIEpAIBAAKCAQEA7x9Kq2mZ\nvL4pR8tN1wB6==\n-----END RSA PRIVATE KEY-----",
  },
  {
    kind: "jwt",
    text: "Cookie: session=eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U",
    secret: "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U",
  },
  { kind: "authorization", text: "Authorization: Bearer 9f2c7Kq2mZ9vL4pR8tN1wB6x", secret: "9f2c7Kq2mZ9vL4pR8tN1wB6x" },
  { kind: "authorization", text: `Authorization: Basic ${Buffer.from("deploy:Zq8Lm2Vx9Rt").toString("base64")}`, secret: Buffer.from("deploy:Zq8Lm2Vx9Rt").toString("base64") },
];

describe("scrubSecrets", () => {
  it.each(PATTERN_SECRETS)("redacts a $kind with a marker that names it and hashes it, and keeps nothing of it", ({ kind, text, secret }) => {
    const scrubbed = scrubSecrets(text);
    expect(scrubbed).toContain(`[redacted:${kind}:${hash(secret)}]`);
    expect(scrubbed).not.toContain(secret);
    // No prefix of the secret survives beside the marker.
    expect(scrubbed).not.toContain(secret.slice(0, 8));
    expect(scrubSecrets(scrubbed)).toBe(scrubbed);
  });

  it("gives one secret the same marker wherever it appears, and another secret another", () => {
    const first = GITHUB_PAT_CLASSIC;
    const second = fake("ghp_", "9Z8y7X6w5V4u3T2s1R0qPoNmLkJiHgFeDcBa");
    const once = scrubSecrets(`a ${first}`);
    expect(scrubSecrets(`b ${first} c`)).toContain(once.slice(2));
    expect(scrubSecrets(second)).not.toBe(once.slice(2));
  });

  it("redacts only the password of a URL, never a placeholder there", () => {
    expect(scrubSecrets("DATABASE_URL=postgres://app:Zq8Lm2Vx9Rt4@db.internal:5432/app"))
      .toBe(`DATABASE_URL=postgres://app:[redacted:url-password:${hash("Zq8Lm2Vx9Rt4")}]@db.internal:5432/app`);
    expect(scrubSecrets("postgres://app:${DB_PASSWORD}@db/app")).toBe("postgres://app:${DB_PASSWORD}@db/app");
    expect(scrubSecrets("git clone git@github.com:acme/app.git && open https://github.com/acme/app")).toBe(
      "git clone git@github.com:acme/app.git && open https://github.com/acme/app",
    );
  });

  it("redacts a cut-off private key as far as its base64 runs", () => {
    const scrubbed = scrubSecrets("-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAABG5vbmU (output truncated)");
    expect(scrubbed).toMatch(/^\[redacted:private-key:[0-9a-f]{6}\]\(output truncated\)$/u);
  });

  it("redacts a value assigned to a secret-like key without a hash, in every assignment form", () => {
    for (const [text, expected] of [
      ["DB_PASSWORD=Tr0ub4dor&3xyzQ", "DB_PASSWORD=[redacted:password]"],
      ["password: mysecretpassword", "password: [redacted:password]"],
      ['"client_secret": "a8Kz2mQ9vLp4Rt7X"', '"client_secret": "[redacted:secret]"'],
      ["api_key = 'Zx9qL2mK8vP4rT7w'", "api_key = '[redacted:api-key]'"],
      ["  authToken: Zx9qL2mK8vP4rT7wQ", "  authToken: [redacted:token]"],
      ["$config = ['db_pwd' => 'hunter2Hunter9!'];", "$config = ['db_pwd' => '[redacted:password]'];"],
    ] as const) expect(scrubSecrets(text), text).toBe(expected);
  });

  it("leaves placeholders, short or repetitive values and code under a secret-like key", () => {
    for (const text of [
      "password: changeme_please",
      "password = ${PASSWORD}",
      "token: <your-token-here>",
      "secret: ************",
      "API_KEY=xxxxxxxxxxxxxxxx",
      "password=hunter2",
      "password=aaaaaaaaaaaaaaaa",
      "token: process.env.GITHUB_TOKEN_VALUE",
      "apiKey: config.get('apiKey')",
      "password: z.string().min(12)",
      "token: string;",
      "def login(password: Optional[str] = None):",
    ]) expect(scrubSecrets(text), text).toBe(text);
  });

  it("never redacts hashes, UUIDs, integrity strings or data URIs, even under a secret-like key", () => {
    for (const text of [
      'token = "a94a8fe5ccb19ba61c4c0873d391e987982fbbd3"',
      "secret: 9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08",
      "session_token: 3f2b8c1e-4d5a-4b6c-8e7f-9a0b1c2d3e4f",
      'auth: "sha512-RdJUflcE3cUzKiMqQgsCu06FPu9UdIJO0beYbPhHN4k6apgJtifcoCtT9bcxOpYBtpD2kCM6Sbzg4CausW/PKQ=="',
      'token: "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg=="',
    ]) expect(scrubSecrets(text), text).toBe(text);
  });

  it("leaves a lockfile, a git log and minified code as they are", () => {
    const packageLock = JSON.stringify({
      name: "app",
      lockfileVersion: 3,
      packages: {
        "node_modules/js-tokens": {
          version: "4.0.0",
          resolved: "https://registry.npmjs.org/js-tokens/-/js-tokens-4.0.0.tgz",
          integrity: "sha512-RdJUflcE3cUzKiMqQgsCu06FPu9UdIJO0beYbPhHN4k6apgJtifcoCtT9bcxOpYBtpD2kCM6Sbzg4CausW/PKQ==",
        },
        "node_modules/@octokit/auth-token": {
          version: "4.0.0",
          resolved: "https://registry.npmjs.org/@octokit/auth-token/-/auth-token-4.0.0.tgz",
          integrity: "sha512-tY/msAuJo6ARbK6SPIxZrPBms3xPbfwBrulZe0Wtr/DIY9lje2HeV1uoebShn6mx7SjCHif6EjMvoREj+gZ+SA==",
          engines: { node: ">= 18" },
        },
      },
    }, null, 2);
    const pnpmLock = [
      "packages:",
      "  /@octokit/auth-token@4.0.0:",
      "    resolution: {integrity: sha512-tY/msAuJo6ARbK6SPIxZrPBms3xPbfwBrulZe0Wtr/DIY9lje2HeV1uoebShn6mx7SjCHif6EjMvoREj+gZ+SA==}",
      "    engines: {node: '>= 18'}",
      "  /jsonwebtoken@9.0.2:",
      "    resolution: {integrity: sha512-PRp66vJ865SSqOlgqS8hujT5U4AOgMfhrwYIuIhfKaoQmfQnkYa/jl7sdd+L/iDENbBoB5O05hWhSlPRZE5saQ==}",
    ].join("\n");
    const yarnLock = [
      '"@auth/core@0.18.0":',
      '  version "0.18.0"',
      '  resolved "https://registry.yarnpkg.com/@auth/core/-/core-0.18.0.tgz#4f4ba8b4d5a3cbe85c8fa3b3d1d9e9f2a0b2c8d1"',
      "  integrity sha512-QnGLtZp1Dcjx0hYmxfBnHq5zBShbKLbK4Ej3KJ3gQ2k7aOBJiZ9i8dL2eZVJGk7ZgQ6t5yIPw8q7x4XlYmQhTQ==",
    ].join("\n");
    const gitLog = [
      "commit 3b18e512dba79e4c8300dd08aeb37f8e728b8dad",
      "Merge: 1a2b3c4 5d6e7f8",
      "Author: Jane Doe <jane@example.com>",
      "Date:   Mon Oct 27 10:00:00 2026 +0000",
      "",
      "    Rotate auth token handling; rename password field",
      "",
      "    Change-Id: I8473b95934b5732ac55d26311a706c9c2bde9940",
      "    Signed-off-by: Jane Doe <jane@example.com>",
    ].join("\n");
    const minified = 'function n(e){var t=e.token,r=e.password||"",o={apiKey:n.apiKey||"",secret:t,auth:function(e){return e.authToken}};'
      + 'return fetch(u,{headers:{Authorization:"Bearer "+t}})}var a="0123456789abcdef0123456789abcdef",i=crypto.randomUUID(),'
      + "s={accessKeyId:c.accessKeyId,secretAccessKey:c.secretAccessKey,sessionToken:c.sessionToken};";
    for (const text of [packageLock, pnpmLock, yarnLock, gitLog, minified, "Bearer authentication is described in RFC 6750"]) {
      expect(scrubSecrets(text)).toBe(text);
    }
  });

  it("replaces a string too long to scan whole, and stays linear on many unterminated key headers", () => {
    expect(scrubSecrets("a".repeat(MAX_SCRUB_CHARS + 1))).toBe("[redacted:unscanned]");
    const headers = "-----BEGIN RSA PRIVATE KEY-----!".repeat(100_000);
    const started = performance.now();
    expect(scrubSecrets(headers)).not.toContain("-----BEGIN");
    expect(performance.now() - started).toBeLessThan(5_000);
  });

  it("tells whether text holds a secret", () => {
    expect(containsSecret(`API_KEY = "${OPENAI_PROJECT}"`)).toBe(true);
    expect(containsSecret("const answer = 42;")).toBe(false);
  });
});

describe("scrubValue", () => {
  it("scrubs every string of a payload, judging a string under a secret-like key as an assignment", () => {
    const payload = {
      kind: "call",
      input: { command: "curl -H 'Authorization: Bearer 9f2c7Kq2mZ9vL4pR8tN1wB6x' https://api", env: { API_TOKEN: "Zx9qL2mK8vP4rT7wQ" } },
      outputs: ["ok", GITLAB],
      count: 3,
    };
    expect(scrubValue(payload)).toEqual({
      kind: "call",
      input: {
        command: `curl -H 'Authorization: Bearer [redacted:authorization:${hash("9f2c7Kq2mZ9vL4pR8tN1wB6x")}]' https://api`,
        env: { API_TOKEN: "[redacted:token]" },
      },
      outputs: ["ok", `[redacted:gitlab-token:${hash(GITLAB)}]`],
      count: 3,
    });
    expect(payload.outputs[1]).toBe(GITLAB);
  });

  it("returns a payload without secrets as it is, cycles included, and keeps a scrubbed copy's cycles", () => {
    const clean: Record<string, unknown> = { text: "hello", count: 7n };
    clean.self = clean;
    expect(scrubValue(clean)).toBe(clean);
    const leaky: Record<string, unknown> = { text: GITHUB_PAT_CLASSIC };
    leaky.self = leaky;
    const scrubbed = scrubValue(leaky);
    expect(scrubbed.self).toBe(scrubbed);
    expect(scrubbed.text).toMatch(/^\[redacted:github-token:/u);
  });

  it("keeps an own __proto__ key a key", () => {
    const parsed = JSON.parse(`{"__proto__": {"note": "${GITHUB_PAT_CLASSIC}"}}`) as Record<string, unknown>;
    const scrubbed = scrubValue(parsed);
    expect(Object.getPrototypeOf(scrubbed)).toBe(Object.prototype);
    expect(JSON.stringify(scrubbed)).toContain("[redacted:github-token:");
  });
});
