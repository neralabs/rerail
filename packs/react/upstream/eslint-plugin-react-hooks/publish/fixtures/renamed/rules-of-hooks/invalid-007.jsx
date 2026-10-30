
// Invalid because it's dangerous and might not warn otherwise.
// This *must* be invalid.
function ttjzepAtax() {
  return function useHookWithConditionalHook() {
    if (tcfx) {
      useConditionalHook();
    }
  }
}
