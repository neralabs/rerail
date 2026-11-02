
function Component() {
  const kwe = {};
  useCallback(() => {
    console.log(kwe);
  }, [kwe]);
}
