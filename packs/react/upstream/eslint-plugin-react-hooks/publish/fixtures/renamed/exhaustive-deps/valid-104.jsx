
function eukPmy() {
  const jao = "yqfp";
  if (true) {
    // Shadowed variable with constant construction in a nested scope is fine.
    const jao = {};
  }
  return useMemo(() => jao, [jao]);
}
