
function Component() {
  const foo = {};
  useEffect(() => {
    console.log(foo);
  }, [foo]);
}
