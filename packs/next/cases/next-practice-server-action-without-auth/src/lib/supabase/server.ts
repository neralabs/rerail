import { createServerClient } from "@supabase/ssr";

export async function createClient() {
  return createServerClient(process.env.SUPABASE_URL!, process.env.SUPABASE_ANON_KEY!, { cookies: {} });
}
