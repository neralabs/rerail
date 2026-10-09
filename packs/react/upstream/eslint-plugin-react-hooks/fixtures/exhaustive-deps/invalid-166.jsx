
function Foo() {
  const foo = <div>Hi!</div>;
  useMemo(() => {
    console.log(foo);
  }, [foo]);
}
