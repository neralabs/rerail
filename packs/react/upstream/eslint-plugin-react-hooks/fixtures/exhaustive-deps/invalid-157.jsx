
function Component() {
  let foo = {};
  useMemo(() => foo, [foo]);
}
