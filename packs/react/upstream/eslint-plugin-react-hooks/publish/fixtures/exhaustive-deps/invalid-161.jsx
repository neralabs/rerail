
function Component() {
  const foo = {};
  useLayoutEffect(() => {
    console.log(foo);
  }, [foo]);
}
