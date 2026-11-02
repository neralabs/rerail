
function Component() {
  const foo = class {};
  useMemo(() => foo, [foo]);
}
