
function Foo() {
  const foo = bar = {};
  useMemo(() => {
    console.log(foo);
  }, [foo]);
}
