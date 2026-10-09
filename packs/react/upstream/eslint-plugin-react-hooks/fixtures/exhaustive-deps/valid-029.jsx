
function MyComponent(props) {
  useCallback(() => {
    console.log(props.foo.bar?.toString());
  }, [props.foo.bar]);
}
