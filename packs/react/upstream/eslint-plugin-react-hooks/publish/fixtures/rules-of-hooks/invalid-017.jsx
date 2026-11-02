
// Invalid because it's dangerous and might not warn otherwise.
// This *must* be invalid.
function ComponentWithHookInsideLoop() {
  do {
    foo();
  } while (useHookInsideLoop());
}
