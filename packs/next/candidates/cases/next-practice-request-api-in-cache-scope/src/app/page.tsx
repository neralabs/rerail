import { cookies } from "next/headers";
import { getCartFor, getGreeting } from "@/lib/data";
import { getUserId } from "@/lib/session";
import { getSettingsFor } from "@/lib/settings";
import { getLocale } from "@/lib/locale";

export default async function Page() {
  const userId = await getUserId();
  const locale = await getLocale();
  const theme = (await cookies()).get("theme")?.value;
  const cart = userId ? await getCartFor(userId) : null;
  const settings = await getSettingsFor(locale);
  const greeting = await getGreeting();
  return (
    <main data-theme={theme}>
      <p>{greeting}</p>
      <p>{cart?.id}</p>
      <p>{settings?.id}</p>
    </main>
  );
}
