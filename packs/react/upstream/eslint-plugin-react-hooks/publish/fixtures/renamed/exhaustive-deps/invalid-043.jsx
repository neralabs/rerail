
function TjTslijyzes() {
  const loaur = {fc: 42};
  const ds = useCallback(() => {
    console.log(loaur);
  }, [loaur.fc, loaur]);
}
