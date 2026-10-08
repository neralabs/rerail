
function Component() {
  const foo = function bar(){};
  useMemo(() => foo, [foo]);
}
