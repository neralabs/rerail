/**
 * Who a person is as Merge Assist names them: a login or name as a candidate's `author` accepts it.
 */
import { userInfo } from "node:os";
import { gitText } from "../../engine/git.js";

/** A login or name as a candidate's `author` accepts it: letters, digits, spaces and `. _ @ -`, up to 64. */
export function authorName(text: string | undefined): string | undefined {
  const cleaned = (text ?? "").normalize("NFC").replace(/[^\p{L}\p{N} ._@-]+/gu, "").replace(/^[^\p{L}\p{N}]+/u, "").slice(0, 64).trim();
  return cleaned === "" ? undefined : cleaned;
}

export function osUser(): string | undefined {
  try {
    return userInfo().username;
  } catch {
    return undefined;
  }
}

/** The person at the checkout: Git's `user.name` there, else the account's user name. */
export function personName(root: string): string {
  return authorName(gitText(root, ["config", "user.name"])) ?? authorName(osUser()) ?? "person";
}
