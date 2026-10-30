
// Invalid because it's dangerous and might not warn otherwise.
// This *must* be invalid.
function useHookInLoops() {
  do {
    useHook1();
    if (b) return;
    useHook2();
  } while (d);

  do {
    useHook3();
    if (r) return;
    useHook4();
  } while (s)
}
