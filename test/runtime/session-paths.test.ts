import { join, resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { bundledModelPath } from "../../src/engine/index.js";
import { isDefaultStorageRoot, resolveModelPath, resolveSessionPaths } from "../../src/runtime/session-paths.js";

const workspace = resolve("/", "work", "app");

describe("a session's storage root and model path", () => {
  it("default to the workspace's .rerail/sessions and the bundled model when nothing names them", () => {
    expect(resolveSessionPaths(workspace, {}, {})).toEqual({
      storageRoot: join(workspace, ".rerail", "sessions"),
      modelPath: bundledModelPath(),
    });
  });

  it("treat an empty variable as unset", () => {
    expect(resolveSessionPaths(workspace, {}, { RERAIL_STORAGE_ROOT: "", RERAIL_MODEL_PATH: "" })).toEqual({
      storageRoot: join(workspace, ".rerail", "sessions"),
      modelPath: bundledModelPath(),
    });
  });

  it("resolve a relative variable against the session's workspace, not the process's folder", () => {
    expect(resolveSessionPaths(workspace, {}, { RERAIL_STORAGE_ROOT: "records", RERAIL_MODEL_PATH: "../model" })).toEqual({
      storageRoot: join(workspace, "records"),
      modelPath: resolve("/", "work", "model"),
    });
  });

  it("keep an absolute variable as it is", () => {
    const environment = { RERAIL_STORAGE_ROOT: resolve("/", "records"), RERAIL_MODEL_PATH: resolve("/", "models", "m") };
    expect(resolveSessionPaths(workspace, {}, environment)).toEqual({ storageRoot: resolve("/", "records"), modelPath: resolve("/", "models", "m") });
  });

  it("prefer the caller's paths to the variables, resolving them the same way", () => {
    const environment = { RERAIL_STORAGE_ROOT: resolve("/", "records"), RERAIL_MODEL_PATH: resolve("/", "models", "m") };
    expect(resolveSessionPaths(workspace, { storageRoot: "mine", modelPath: "model" }, environment)).toEqual({
      storageRoot: join(workspace, "mine"),
      modelPath: join(workspace, "model"),
    });
    expect(resolveModelPath(workspace, "", environment)).toBe(resolve("/", "models", "m"));
  });

  it("say whether a root is the workspace's default, whatever its folder is named", () => {
    expect(isDefaultStorageRoot(join(workspace, ".rerail", "sessions"), workspace)).toBe(true);
    expect(isDefaultStorageRoot(join(workspace, ".rerail", "sessions", "..", "sessions"), `${workspace}/`)).toBe(true);
    // Inside a `.rerail` folder, but not this workspace's default.
    expect(isDefaultStorageRoot(join(workspace, ".rerail", "records"), workspace)).toBe(false);
    expect(isDefaultStorageRoot(resolve("/", "other", ".rerail", "sessions"), workspace)).toBe(false);
  });
});
