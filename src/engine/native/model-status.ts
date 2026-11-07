import { bundledModelPath } from "../package-root.js";
import { loadNativeBinding, type NativeModelCheck } from "./native.js";

/**
 * Whether the bundled model's files match `model/manifest.json`, as a session's attach checks them. `message` is what
 * `rerail status`, `/healthz` and the dashboard show: `ok`, or the problem and the file at fault, such as
 * `checksum mismatch (model.safetensors)` for a checkout that holds a Git LFS pointer in place of the weights.
 */
export type ModelStatus =
  | { ok: true; message: "ok" }
  | { ok: false; problem: NonNullable<NativeModelCheck["problem"]> | "unverified"; file: string | null; message: string };

const checked = new Map<string, ModelStatus>();

/**
 * The model in `root` (the bundled `model/` by default) against its manifest. Hashing the files reads about 8 MB, so
 * each root is checked once per process: a model repaired while the daemon runs shows as such after it restarts.
 */
export function modelStatus(
  root: string = bundledModelPath(),
  verify: (root: string) => NativeModelCheck = path => loadNativeBinding().verifyModelFiles(path),
): ModelStatus {
  let status = checked.get(root);
  if (status === undefined) {
    status = verifiedStatus(root, verify);
    checked.set(root, status);
  }
  return status;
}

function verifiedStatus(root: string, verify: (root: string) => NativeModelCheck): ModelStatus {
  let check: NativeModelCheck;
  try {
    check = verify(root);
  } catch (error) {
    // Without the engine nothing can check the files; the engine's own status says why.
    return { ok: false, problem: "unverified", file: null, message: `not verified: ${(error as Error).message}` };
  }
  const { problem } = check;
  if (problem === undefined || problem === null) return { ok: true, message: "ok" };
  const file = check.file ?? null;
  const words = problem.replaceAll("-", " ");
  return { ok: false, problem, file, message: file === null ? words : `${words} (${file})` };
}

/**
 * The `Model:` line of `rerail status`. `model` is what the daemon's `/healthz` answered; a daemon from before model
 * checks answers none.
 */
export function modelStatusLine(model: Pick<ModelStatus, "ok" | "message"> | undefined): string {
  return `Model: ${model?.message ?? "not reported by this daemon (restart it with rerail daemon stop, then rerail daemon start)"}`;
}
