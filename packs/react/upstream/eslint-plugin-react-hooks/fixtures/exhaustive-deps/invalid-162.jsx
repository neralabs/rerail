
function Component() {
  const foo = {};
  useImperativeHandle(
    ref,
    () => {
       console.log(foo);
    },
    [foo]
  );
}
