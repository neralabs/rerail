/** Replay unchanged observations on the legacy owner API and the integrated public API. */
import { loadNativeBinding, type NativeCodeRuleDslExecution } from "../../src/engine/index.js";

export function replayExecution(): NativeCodeRuleDslExecution {
  const native = loadNativeBinding();
  if (typeof native.CodeRuleDslExecution === "function") return new native.CodeRuleDslExecution();
  // The integrated production addon owns snapshots on the compiled rule and
  // resident state on the workspace. This changes API plumbing, not scoring.
  return {
    createWorkspace: () => new native.CodeRuleDslWorkspace(),
    evaluate: (rule, input, mode) => {
      if (mode !== "decision" && mode !== "report") throw new Error(`Unsupported evaluation mode: ${mode}`);
      return rule.evaluate(input, mode);
    },
    close: () => {},
  };
}
