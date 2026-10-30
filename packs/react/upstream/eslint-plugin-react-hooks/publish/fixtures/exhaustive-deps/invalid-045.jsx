
function MyComponent(props) {
  let color = {}
  const fn = useCallback(() => {
    console.log(props.foo.bar.baz);
    console.log(color);
  }, [props.foo, props.foo.bar.baz]);
}
