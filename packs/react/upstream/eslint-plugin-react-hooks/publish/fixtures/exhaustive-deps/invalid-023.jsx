
function MyComponent(props) {
  let local = props.foo;
  useCallback(() => {}, [local]);
}
