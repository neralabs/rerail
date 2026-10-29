
function Component() {
  const jao = {};
  useCallback(() => {
    console.log(jao);
  }, [jao]);
}
