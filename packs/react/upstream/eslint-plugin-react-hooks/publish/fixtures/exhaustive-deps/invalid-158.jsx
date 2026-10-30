
function Component() {
  var foo = {};
  useMemo(() => foo, [foo]);
}
