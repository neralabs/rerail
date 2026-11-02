
function Component() {
  const foo = {};
  useCallback(() => {
    console.log(foo);
  }, [foo]);
}
