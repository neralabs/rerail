// scripts/s3/pack-data-publish.mjs: the publish workflow's sticky comment.

import { describe, expect, it } from "vitest";
import { publishComment } from "../../scripts/s3/pack-data-publish.mjs";

describe("the failed comment", () => {
  const failedLine = (outcomes) => publishComment("failed", { runUrl: "https://run", objects: [], outcomes }).split("\n").find((line) => line.startsWith("- "));

  it("blames approval only when the upload job never ran, and the step when it ran or its credentials failed", () => {
    expect(failedLine({ credentials: "", upload: "" })).toBe("- upload did not run: approval rejected or job cancelled");
    expect(failedLine({ credentials: "failure", upload: "skipped" })).toBe("- upload step failed: see the run");
    expect(failedLine({ credentials: "success", upload: "failure" })).toBe("- upload step failed: see the run");
    expect(failedLine({ credentials: "skipped", upload: "skipped" })).toBe("- upload job failed before its upload step: see the run");
  });
});
