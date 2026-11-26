import { describe, expect, it } from "vitest";

describe("env", () => {
  it("rejects a public service role key", () => {
    process.env.NEXT_PUBLIC_SUPABASE_SERVICE_ROLE_KEY = "x";
    expect(process.env.NEXT_PUBLIC_SUPABASE_SERVICE_ROLE_KEY).toBe("x");
  });
});
