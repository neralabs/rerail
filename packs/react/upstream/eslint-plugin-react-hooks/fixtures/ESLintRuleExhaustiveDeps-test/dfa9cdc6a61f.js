
function Foo() {
  const foo = ({}: any);
  useMemo(() => {
    console.log(foo);
  }, [foo]);
}
