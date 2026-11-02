
function useFoo() {
  const foo = "fine";
  if (true) {
    // Shadowed variable with constant construction in a nested scope is fine.
    const foo = {};
  }
  return useMemo(() => foo, [foo]);
}
