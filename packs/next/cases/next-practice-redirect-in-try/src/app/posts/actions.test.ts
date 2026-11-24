import { redirect } from "next/navigation";
import { expect, it } from "vitest";

it("redirect throws", () => {
  try {
    redirect("/posts");
  } catch (error) {
    expect(String(error)).toContain("NEXT");
  }
});
