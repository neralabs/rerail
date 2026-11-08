import { join, resolve } from "node:path";
import { bundledModelPath } from "../engine/package-root.js";

/** Where a session's records live and which embedding model it loads, resolved once where the session starts. */
export interface SessionPaths {
  storageRoot: string;
  modelPath: string;
}

/** `<workspace>/.rerail/sessions`: the storage root when neither the caller nor `RERAIL_STORAGE_ROOT` names one. */
export function defaultStorageRoot(workspaceRoot: string): string {
  return join(resolve(workspaceRoot), ".rerail", "sessions");
}

/**
 * Whether `storageRoot` is `workspaceRoot`'s default, so the folder is Rerail's alone. A custom root may hold the
 * person's own files.
 */
export function isDefaultStorageRoot(storageRoot: string, workspaceRoot: string): boolean {
  return resolve(storageRoot) === defaultStorageRoot(workspaceRoot);
}

/**
 * The session's storage root and model path: the caller's value, else `RERAIL_STORAGE_ROOT` / `RERAIL_MODEL_PATH`,
 * else {@link defaultStorageRoot} / {@link bundledModelPath}. An empty value counts as unset; a relative one resolves
 * against the session's workspace.
 */
export function resolveSessionPaths(
  workspaceRoot: string,
  given: { storageRoot?: string | undefined; modelPath?: string | undefined } = {},
  environment: NodeJS.ProcessEnv = process.env,
): SessionPaths {
  const storageRoot = setting(given.storageRoot, environment.RERAIL_STORAGE_ROOT);
  return {
    storageRoot: storageRoot === undefined ? defaultStorageRoot(workspaceRoot) : resolve(workspaceRoot, storageRoot),
    modelPath: resolveModelPath(workspaceRoot, given.modelPath, environment),
  };
}

/** The model path alone, as {@link resolveSessionPaths} resolves it. */
export function resolveModelPath(
  workspaceRoot: string,
  given?: string,
  environment: NodeJS.ProcessEnv = process.env,
): string {
  const modelPath = setting(given, environment.RERAIL_MODEL_PATH);
  return modelPath === undefined ? bundledModelPath() : resolve(workspaceRoot, modelPath);
}

function setting(given: string | undefined, variable: string | undefined): string | undefined {
  if (given !== undefined && given !== "") return given;
  return variable === undefined || variable === "" ? undefined : variable;
}
