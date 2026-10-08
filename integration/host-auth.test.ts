import { createHash } from "node:crypto";
import { mkdtemp, readFile, rm, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import Database from "better-sqlite3";
import { describe, expect, it } from "vitest";
import { importAccessOnlyAuth } from "../scripts/host-auth.js";
import { OmpHost } from "./omp-host.js";

function token(expires: number): string {
  return `test.${Buffer.from(JSON.stringify({ exp: Math.floor(expires / 1000), "https://api.openai.com/auth": { chatgpt_account_id: "sandbox-account" } })).toString("base64url")}.synthetic`;
}

describe("access-only sandbox credentials", () => {
  it("is consumed by standalone OMP AuthStorage without copying refresh credentials or modifying the source", async () => {
    const cwd = await mkdtemp(join(tmpdir(), "rerail-auth-host-"));
    let host: OmpHost | undefined;
    try {
      const source = join(cwd, "source.db");
      const expires = Math.floor((Date.now() + 3_600_000) / 1000) * 1000;
      const access = token(expires);
      const db = new Database(source);
      db.exec("CREATE TABLE auth_credentials (id INTEGER PRIMARY KEY, provider TEXT, credential_type TEXT, data TEXT, disabled_cause TEXT)");
      const insert = db.prepare("INSERT INTO auth_credentials VALUES (?, 'openai-codex', 'oauth', ?, ?)");
      insert.run(1, JSON.stringify({ access: token(expires + 100_000), refresh: "disabled-refresh-secret" }), "revoked");
      insert.run(2, JSON.stringify({ access, refresh: "original-refresh-secret", expires, accountId: "sandbox-account" }), null);
      db.close();
      const before = await readFile(source);
      const authPath = join(cwd, "sandbox-auth.db");
      await importAccessOnlyAuth("omp", source, authPath, 180_000);
      expect((await stat(authPath)).mode & 0o777).toBe(0o600);
      host = await OmpHost.start(cwd, [], { extension: "none", authPath });
      const evidence = await host.snapshot();
      expect(evidence.auth).toEqual({
        type: "oauth", accessHash: createHash("sha256").update(access).digest("hex"),
        refresh: "", expires, accountId: "sandbox-account",
      });
      expect(host.errors.join("")).not.toContain(access);
      expect(JSON.stringify(host.frames)).not.toContain(access);
      expect(await readFile(source)).toEqual(before);
      expect((await readFile(authPath)).includes(Buffer.from("original-refresh-secret"))).toBe(false);
      expect((await readFile(authPath)).includes(Buffer.from("disabled-refresh-secret"))).toBe(false);

      const piPath = join(cwd, "pi-auth.json");
      await importAccessOnlyAuth("pi", source, piPath, 180_000);
      expect(JSON.parse(await readFile(piPath, "utf8"))).toEqual({ "openai-codex": {
        type: "oauth", access, refresh: "", expires, accountId: "sandbox-account",
      } });
      expect((await stat(piPath)).mode & 0o777).toBe(0o600);
      const codexSource = join(cwd, "source-codex.json");
      const idToken = token(expires + 60_000);
      const codexOriginal = JSON.stringify({ tokens: { access_token: access, refresh_token: "codex-refresh-secret", id_token: idToken, account_id: "sandbox-account" } });
      await writeFile(codexSource, codexOriginal, { mode: 0o600 });
      const codexPath = join(cwd, "codex-auth.json");
      await importAccessOnlyAuth("codex", codexSource, codexPath, 180_000);
      expect(JSON.parse(await readFile(codexPath, "utf8"))).toMatchObject({ tokens: {
        access_token: access, refresh_token: "", id_token: idToken, account_id: "sandbox-account",
      } });
      expect((await stat(codexPath)).mode & 0o777).toBe(0o600);
      expect(await readFile(codexSource, "utf8")).toBe(codexOriginal);
    } finally {
      try { await host?.close(); } finally { await rm(cwd, { recursive: true, force: true }); }
    }
  }, 90_000);

  it("rejects credentials that expire before a custom smoke deadline and refuses source overwrite", async () => {
    const cwd = await mkdtemp(join(tmpdir(), "rerail-auth-expiry-"));
    try {
      const source = join(cwd, "auth.json");
      const original = JSON.stringify({ tokens: { access_token: token(Date.now() + 700_000), id_token: token(Date.now() + 700_000), refresh_token: "keep-original" } });
      await writeFile(source, original, { mode: 0o600 });
      await expect(importAccessOnlyAuth("codex", source, join(cwd, "sandbox.json"), 900_000)).rejects.toThrow("smoke timeout");
      await expect(importAccessOnlyAuth("codex", source, source, 1_000)).rejects.toThrow("must differ");
      expect(await readFile(source, "utf8")).toBe(original);
    } finally { await rm(cwd, { recursive: true, force: true }); }
  });
});
