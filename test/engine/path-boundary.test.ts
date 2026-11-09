import { mkdirSync, mkdtempSync, realpathSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import {
  checkoutPathBoundary, deniedByPathBoundary, SENSITIVE_FOLDER_NAMES, sensitiveByName,
} from "../../src/engine/path-boundary.js";

const roots: string[] = [];
afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

/** A checkout with a `.env`, a tracked file linked to it, and an ordinary file. */
function checkout(): string {
  const root = realpathSync(mkdtempSync(join(tmpdir(), "rerail-path-boundary-")));
  roots.push(root);
  writeFileSync(join(root, ".env"), "API_KEY=x\n");
  mkdirSync(join(root, "docs"));
  writeFileSync(join(root, "docs", "guide.md"), "# Guide\n");
  symlinkSync("../.env", join(root, "docs", "a.md"));
  return root;
}

describe("the path boundary", () => {
  it("judges a file secret-like by every family of name it lists, in any case", () => {
    for (const path of [
      ".env",
      "web/.env.local",
      "config/prod.env",
      ".npmrc",
      ".pgpass",
      "id_rsa",
      "keys/id_ed25519.pub",
      "certs/server.pem",
      "certs/server.key",
      "store.p12",
      "terraform.tfstate",
      "terraform.tfstate.backup",
      "prod.tfvars",
      "appsettings.Production.json",
      "deploy/service-account.json",
      "deploy/service-account-prod.json",
      "credentials",
      "credentials.json",
      "credentials-prod.json",
      "config/credentials_old.yml",
      "secrets.yaml",
      "auth.json",
      "CERTS/SERVER.PEM",
      "Credentials-Prod.JSON",
      ".ENV",
      "token",
      "config/token.txt",
      "auth/api.token",
      "config/app-secrets.yaml",
      "client_secret.json",
      "deploy/serviceAccountKey.json",
      "deploy/serviceaccount.json",
      "gcp-key.json",
      "deploy_key.json",
      "kubeconfig",
      "clusters/prod.kubeconfig",
      "kubeconfig.yaml",
      "terraform.tfvars.json",
      "prod.sops.yaml",
      ".git-credentials",
      "home/.bash_history",
      ".zsh_history",
    ]) expect(sensitiveByName(path), path).toBe(true);
  });

  it("judges any path through a credential folder secret-like, whatever the file is called", () => {
    for (const folder of SENSITIVE_FOLDER_NAMES) {
      expect(sensitiveByName(`home/${folder}/config`), folder).toBe(true);
      expect(sensitiveByName(`${folder.toUpperCase()}/notes.txt`), folder).toBe(true);
    }
  });

  it("leaves source named near a secret alone", () => {
    for (const path of [
      "apem",
      "src/keyboard.ts",
      "src/monkey.key.ts",
      "src/password-validator.ts",
      "src/credentials-helper.ts",
      "src/credentialsStore.ts",
      "src/environment.ts",
      "src/valid_rsa.ts",
      "appsettings.json",
      "service-account.yaml",
      "terraform.tf",
      "src/secret-manager.ts",
      "src/token-parser.py",
      "src/token.ts",
      "src/tokens.ts",
      // Design tokens (Style Dictionary's `tokens.json`) are not credentials.
      "design/tokens.json",
      "src/keyboard.json",
      "keymap.json",
      // A tokenizer's vocabulary (Rerail's own model ships one) is data, not a credential.
      "model/tokenizer.json",
      "src/kubeconfig.ts",
      "src/history.ts",
      "src/sops.ts",
    ]) expect(sensitiveByName(path), path).toBe(false);
  });

  it("denies a hidden path too, which by its name alone is not secret-like", () => {
    expect(sensitiveByName(".github/workflows/ci.yml")).toBe(false);
    expect(deniedByPathBoundary(".github/workflows/ci.yml")).toBe(true);
    expect(deniedByPathBoundary("src/.hidden/a.ts")).toBe(true);
    expect(deniedByPathBoundary("config/secrets.yaml")).toBe(true);
    expect(deniedByPathBoundary("src/app.ts")).toBe(false);
  });

  it("judges a path by where it leads in a checkout, and by its name where it does not resolve", () => {
    const root = checkout();
    expect(sensitiveByName("docs/a.md")).toBe(false);
    const boundary = checkoutPathBoundary(root);
    expect(boundary.leadsTo("docs/a.md")).toBe(".env");
    expect(boundary.sensitive("docs/a.md")).toBe(true);
    expect(boundary.sensitive(join(root, "docs", "a.md"))).toBe(true);
    expect(boundary.leadsTo("docs/guide.md")).toBeUndefined();
    expect(boundary.sensitive("docs/guide.md")).toBe(false);
    expect(boundary.sensitive("docs/missing.md")).toBe(false);
    expect(boundary.sensitive("config/secrets.yaml")).toBe(true);
  });

  it("judges a link out of the checkout by the absolute path it leads to", () => {
    const root = checkout();
    const outside = realpathSync(mkdtempSync(join(tmpdir(), "rerail-path-boundary-home-")));
    roots.push(outside);
    mkdirSync(join(outside, ".aws"));
    writeFileSync(join(outside, ".aws", "credentials"), "[default]\n");
    symlinkSync(join(outside, ".aws", "credentials"), join(root, "docs", "b.md"));
    expect(checkoutPathBoundary(root).sensitive("docs/b.md")).toBe(true);
  });

  it("judges by name alone past its resolutions, and in a checkout that does not exist", () => {
    const root = checkout();
    const spent = checkoutPathBoundary(root, 1);
    expect(spent.sensitive("docs/guide.md")).toBe(false);
    expect(spent.sensitive("docs/a.md")).toBe(false);
    expect(checkoutPathBoundary(join(root, "gone")).sensitive("docs/a.md")).toBe(false);
  });
});
