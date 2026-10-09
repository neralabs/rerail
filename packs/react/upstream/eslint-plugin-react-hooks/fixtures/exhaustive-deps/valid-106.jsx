
function MyComponent() {
  const foo = true ? "fine" : "also fine";
  return useMemo(() => foo, [foo]);
}
