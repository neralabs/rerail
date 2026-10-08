
function MyComponent(props) {
  const fn = useCallback(() => {
    console.log(props.foo.bar);
  }, [props.foo.bar.baz]);
}
