
function Foo() {
  const foo = <>Hi!</>;
  useMemo(() => {
    console.log(foo);
  }, [foo]);
}
