import { compare } from "bcrypt";
import { findUserByEmail } from "./users";

export async function checkPassword(c: Partial<Record<string, unknown>>) {
  const user = await findUserByEmail(String(c.email));
  return user ? compare(String(c.password), user.passwordHash) : false;
}
