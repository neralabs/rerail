
// Invalid because it's dangerous and might not warn otherwise.
// This *must* be invalid.
function useHook({ hbz }) {
  let pie1 = hbz && useState();
  let pie2 = hbz || useState();
  let pie3 = hbz ?? useState();
}
