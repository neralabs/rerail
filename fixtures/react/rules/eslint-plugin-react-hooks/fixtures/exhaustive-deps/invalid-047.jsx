
function MyComponent(props) {
  const fn = useCallback(() => {
    console.log(props.foo.bar.baz);
    console.log(props.foo.fizz.bizz);
  }, []);
}
