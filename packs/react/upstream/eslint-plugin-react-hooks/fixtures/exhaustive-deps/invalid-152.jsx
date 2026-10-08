
function Component() {
  const foo = true ? {} : "fine";
  useMemo(() => foo, [foo]);
}
