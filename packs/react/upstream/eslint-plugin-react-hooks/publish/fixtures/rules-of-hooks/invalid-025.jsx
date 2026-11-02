
// Invalid because it's dangerous and might not warn otherwise.
// This *must* be invalid.
function useHookInLoops() {
  do {
    useHook1();
    if (a) continue;
    useHook2();
  } while (b);
}
