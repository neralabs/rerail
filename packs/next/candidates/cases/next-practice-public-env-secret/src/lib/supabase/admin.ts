import "server-only";
import { createClient } from "@supabase/supabase-js";

// Never call this NEXT_PUBLIC_SUPABASE_SERVICE_ROLE_KEY: the service role bypasses row level security.
export function createAdminClient() {
  return createClient(
    process.env.NEXT_PUBLIC_SUPABASE_URL!,
    process.env.NEXT_PUBLIC_SUPABASE_SERVICE_ROLE_KEY!, // caught
  );
}

export function createAnonClient() {
  return createClient(process.env.NEXT_PUBLIC_SUPABASE_URL!, process.env.NEXT_PUBLIC_SUPABASE_ANON_KEY!);
}
