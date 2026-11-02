
function Foo() {
  const foo = {} as any;
  useMemo(() => {
    console.log(foo);
  }, [foo]);
}
