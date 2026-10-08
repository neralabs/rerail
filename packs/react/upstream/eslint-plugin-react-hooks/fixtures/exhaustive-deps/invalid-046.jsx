
function MyComponent(props) {
  const fn = useCallback(() => {
    console.log(props.foo.bar.baz);
  }, [props.foo.bar.baz, props.foo]);
}
