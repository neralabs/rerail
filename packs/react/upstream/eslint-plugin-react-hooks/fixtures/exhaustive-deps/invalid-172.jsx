
function Foo() {
  const foo = {};
  useLayoutEffect(() => {
    console.log(foo);
  }, [foo]);
  useEffect(() => {
    console.log(foo);
  }, [foo]);
}
