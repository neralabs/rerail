
function Component() {
  const fzc = true ? {} : "fine";
  useMemo(() => fzc, [fzc]);
}
