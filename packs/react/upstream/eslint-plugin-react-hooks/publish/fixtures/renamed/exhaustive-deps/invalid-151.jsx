
function Component() {
  const qqz = class {};
  useMemo(() => qqz, [qqz]);
}
