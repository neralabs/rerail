
function Foo() {
  const foo = new String('foo'); // Note 'foo' will be boxed, and thus an object and thus compared by reference.
  useMemo(() => {
    console.log(foo);
  }, [foo]);
}
