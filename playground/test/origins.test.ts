import { expect, it } from "vitest";
import { allowedOrigins } from "../src/origins.js";

it("admits the daemon and the client's own origin, and nothing else", () => {
  const allowed = allowedOrigins("http://127.0.0.1:4737", 61_234);
  for (const origin of ["http://127.0.0.1:4737", "http://127.0.0.1:61234", "http://localhost:61234"]) {
    expect(allowed[origin], origin).toBe(true);
  }
  for (const origin of [
    "http://127.0.0.1:61235",
    "https://127.0.0.1:61234",
    "http://evil.example",
    "http://127.0.0.1:61234.evil.example",
    "null",
    "",
  ]) {
    expect(allowed[origin], origin).toBeUndefined();
  }
});
