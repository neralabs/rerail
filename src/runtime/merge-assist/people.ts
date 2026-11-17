/** Who a person is as Merge Assist names them: on a conflict's side, and as who dismissed it. */
import { userInfo } from "node:os";
import { gitReadAsync } from "../../engine/merge-assist/branch-git.js";

/** A name as Merge Assist shows it: letters, digits, spaces and `. _ @ -`, up to 64. */
function cleanName(text: string | undefined): string | undefined {
  const cleaned = (text ?? "").normalize("NFC").replace(/[^\p{L}\p{N} ._@-]+/gu, "").replace(/^[^\p{L}\p{N}]+/u, "").slice(0, 64).trim();
  return cleaned === "" ? undefined : cleaned;
}

function osUser(): string | undefined {
  try {
    return userInfo().username;
  } catch {
    return undefined;
  }
}

/** The person at the checkout: Git's `user.name` there, else the account's user name. Reads Git without blocking. */
export async function personName(root: string): Promise<string> {
  return cleanName(await gitReadAsync(root, ["config", "user.name"])) ?? cleanName(osUser()) ?? "person";
}
