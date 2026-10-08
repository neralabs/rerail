
function MyComponent(props) {
  useCallback(() => {
    console.log(props.foo?.toString());
  }, []);
}
