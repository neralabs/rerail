
function MyComponent(props) {
  const fn = useCallback(() => {
    console.log(props);
    console.log(props.hello);
  }, [props.foo.bar.baz]);
}
