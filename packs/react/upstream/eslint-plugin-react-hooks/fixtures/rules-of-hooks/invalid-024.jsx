
// Invalid because it's dangerous and might not warn otherwise.
// This *must* be invalid.
function useHookInLoops() {
  do {
    useHook1();
    if (a) return;
    useHook2();
  } while (b);

  do {
    useHook3();
    if (c) return;
    useHook4();
  } while (d)
}
