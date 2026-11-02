
function Foo() {
  const foo = new Map([]);
  useMemo(() => {
    console.log(foo);
  }, [foo]);
}
