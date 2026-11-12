/**
 * Who a person is as Merge Assist names them: on a conflict's side, as who dismissed it, and as whose interruption
 * budget the dimmer spends.
 */
import { userInfo } from "node:os";
import { gitText } from "../../engine/git.js";

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

/** The person at the checkout: Git's `user.name` there, else the account's user name. */
export function personName(root: string): string {
  return cleanName(gitText(root, ["config", "user.name"])) ?? cleanName(osUser()) ?? "person";
}
