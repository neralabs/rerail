import { describe, expect, it } from "vitest";
import { deniedByPathBoundary, SENSITIVE_FOLDER_NAMES, sensitiveByName } from "../../src/engine/path-boundary.js";

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
    ]) expect(sensitiveByName(path), path).toBe(false);
  });

  it("denies a hidden path too, which by its name alone is not secret-like", () => {
    expect(sensitiveByName(".github/workflows/ci.yml")).toBe(false);
    expect(deniedByPathBoundary(".github/workflows/ci.yml")).toBe(true);
    expect(deniedByPathBoundary("src/.hidden/a.ts")).toBe(true);
    expect(deniedByPathBoundary("config/secrets.yaml")).toBe(true);
    expect(deniedByPathBoundary("src/app.ts")).toBe(false);
  });
});
