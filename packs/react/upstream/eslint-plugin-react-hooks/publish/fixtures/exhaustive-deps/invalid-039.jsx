
function MyComponent(props) {
  const local = {};
  useCallback(() => {
    console.log(props.foo);
    console.log(props.bar);
  }, [props, props.foo]);
}
