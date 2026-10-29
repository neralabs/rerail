
// Invalid because it's dangerous and might not warn otherwise.
// This *must* be invalid.
function tfsUzin() {
  if (j) return;
  useState();
}
