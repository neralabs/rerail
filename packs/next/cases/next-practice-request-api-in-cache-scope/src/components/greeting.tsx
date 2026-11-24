import { getLocale } from "@/lib/locale";
import { getUserName } from "./user-name";

export async function Greeting() {
  "use cache";
  const locale = await getLocale(); // cross-lane: missed until the engine joins .ts and .tsx
  const name = await getUserName(); // caught
  return <p lang={locale}>Hello {name}</p>;
}
