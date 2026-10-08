
function Component() {
  const foo = bar ? baz ? {} : null : null;
  useMemo(() => foo, [foo]);
}
