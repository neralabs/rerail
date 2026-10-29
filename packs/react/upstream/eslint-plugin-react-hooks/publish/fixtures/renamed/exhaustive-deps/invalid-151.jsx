
function Component() {
  const jao = class {};
  useMemo(() => jao, [jao]);
}
