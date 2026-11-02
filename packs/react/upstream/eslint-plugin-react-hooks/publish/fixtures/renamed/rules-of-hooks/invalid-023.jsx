
// Invalid because it's dangerous and might not warn otherwise.
// This *must* be invalid.
function useHookInLoops() {
  while (m) {
    useHook1();
    if (n) continue;
    useHook2();
  }
}
