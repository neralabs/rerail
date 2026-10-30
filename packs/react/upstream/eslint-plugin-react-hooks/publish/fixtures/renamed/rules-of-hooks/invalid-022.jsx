
// Invalid because it's dangerous and might not warn otherwise.
// This *must* be invalid.
function useHookInLoops() {
  while (d) {
    useHook1();
    if (k) return;
    useHook2();
  }
  while (m) {
    useHook3();
    if (p) return;
    useHook4();
  }
}
