
// Invalid because it's dangerous and might not warn otherwise.
// This *must* be invalid.
function useHook() {
  l && useHook1();
  q && useHook2();
}
