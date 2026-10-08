
function Component() {
  const foo = () => {};
  useMemo(() => foo, [foo]);
}
