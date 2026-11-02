
function Example({ prop }) {
  const foo = useCallback(() => {
    prop.hello(foo);
  }, [foo]);
  const bar = useCallback(() => {
    foo();
  }, [foo]);
}
