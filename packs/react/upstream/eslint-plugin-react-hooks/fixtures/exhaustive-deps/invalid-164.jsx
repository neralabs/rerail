
function Foo(section) {
  const foo = {};
  console.log(foo);
  useMemo(() => {
    console.log(foo);
  }, [foo]);
}
