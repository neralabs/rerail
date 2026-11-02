
function Foo() {
  const foo = /reg/;
  useMemo(() => {
    console.log(foo);
  }, [foo]);
}
