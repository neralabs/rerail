
// Invalid because it's dangerous and might not warn otherwise.
// This *must* be invalid.
function tfsUzin() {
  if (l) {
    console.log('true');
  } else {
    console.log('false');
  }
  if (j) return;
  useState();
}
