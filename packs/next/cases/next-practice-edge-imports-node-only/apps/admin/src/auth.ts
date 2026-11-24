import NextAuth from "next-auth";
import Credentials from "next-auth/providers/credentials";
import { checkPassword } from "./lib/password";

export const { handlers, auth } = NextAuth({
  providers: [Credentials({ authorize: async (c) => ((await checkPassword(c)) ? { id: String(c.email) } : null) })],
});
