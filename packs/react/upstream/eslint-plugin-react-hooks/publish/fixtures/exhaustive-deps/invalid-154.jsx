
function Component() {
  const foo = bar ?? {};
  useMemo(() => foo, [foo]);
}
