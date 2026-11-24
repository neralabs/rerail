import { cookies } from "next/headers";

export async function getUserName() {
  return (await cookies()).get("name")?.value ?? "there";
}
