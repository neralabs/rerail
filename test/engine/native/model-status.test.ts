import { createHash } from "node:crypto";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { modelStatus, modelStatusLine } from "../../../src/engine/native/model-status.js";

const roots: string[] = [];

/** A model directory whose manifest lists these files' checksums. */
function modelDirectory(files: Record<string, string>): string {
  const root = mkdtempSync(join(tmpdir(), "rerail-model-status-"));
  roots.push(root);
  const listed: Record<string, { sha256: string }> = {};
  for (const [name, content] of Object.entries(files)) {
    writeFileSync(join(root, name), content);
    listed[name] = { sha256: createHash("sha256").update(content).digest("hex") };
  }
  writeFileSync(join(root, "manifest.json"), JSON.stringify({ formatVersion: 1, modelId: "test", revision: "test", dimensions: 1, files: listed }));
  return root;
}

const FILES = { "model.safetensors": "weights", "tokenizer.json": "{}", "config.json": "{}" };

afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

describe("the model's health", () => {
  it("is ok for the bundled model", () => {
    expect(modelStatus()).toEqual({ ok: true, message: "ok" });
  });

  it("names the file whose checksum does not match the manifest, as a Git LFS pointer does", () => {
    const root = modelDirectory(FILES);
    writeFileSync(join(root, "model.safetensors"), "version https://git-lfs.github.com/spec/v1\noid sha256:a4264cfe\nsize 7559256\n");
    const status = modelStatus(root);
    expect(status).toMatchObject({ ok: false, problem: "checksum-mismatch", file: "model.safetensors" });
    expect(modelStatusLine(status)).toBe("Model: checksum mismatch (model.safetensors)");
  });

  it("names a file the manifest lists that is not there", () => {
    const root = modelDirectory(FILES);
    rmSync(join(root, "tokenizer.json"));
    expect(modelStatus(root)).toMatchObject({ ok: false, problem: "missing", file: "tokenizer.json" });
  });

  it("checks each model directory once per process", () => {
    const root = modelDirectory(FILES);
    let checks = 0;
    const verify = () => {
      checks += 1;
      return { problem: null, file: null };
    };
    expect(modelStatus(root, verify)).toEqual({ ok: true, message: "ok" });
    expect(modelStatus(root, verify)).toEqual({ ok: true, message: "ok" });
    expect(checks).toBe(1);
  });

  it("is not ok when nothing can verify the files", () => {
    const root = modelDirectory(FILES);
    const status = modelStatus(root, () => {
      throw new Error("Unsupported platform or missing native addon");
    });
    expect(status).toMatchObject({ ok: false, problem: "unverified", file: null });
  });

  it("does not claim an ok model for a daemon that reports none", () => {
    const line = modelStatusLine(undefined);
    expect(line.startsWith("Model: ")).toBe(true);
    expect(line).not.toBe("Model: ok");
  });
});
