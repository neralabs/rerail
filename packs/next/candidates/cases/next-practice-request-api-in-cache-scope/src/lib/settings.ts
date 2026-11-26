import { unstable_cache } from "next/cache";
import { cookies } from "next/headers";
import { prisma } from "@/lib/db";
import { getLocale } from "@/lib/locale";

export const getTheme = unstable_cache(
  async () => {
    const store = await cookies(); // caught
    return store.get("theme")?.value ?? "light";
  },
  ["theme"],
);

async function loadSettings() {
  const locale = await getLocale(); // caught
  return prisma.settings.findFirst({ where: { locale } });
}

export const getSettings = unstable_cache(loadSettings, ["settings"], { revalidate: 60 });

// The fix for unstable_cache: the value comes in as an argument.
export const getSettingsFor = unstable_cache(
  async (locale: string) => prisma.settings.findFirst({ where: { locale } }),
  ["settings-for"],
);
