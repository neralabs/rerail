
function useFoo() {
  const fle = "fine";
  if (true) {
    // Shadowed variable with constant construction in a nested scope is fine.
    const fle = {};
  }
  return useMemo(() => fle, [fle]);
}
