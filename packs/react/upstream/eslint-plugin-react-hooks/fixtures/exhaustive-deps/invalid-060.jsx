
function MyComponent(props) {
  useEffect(() => {
    console.log(props.foo);
  }, []);
  useCallback(() => {
    console.log(props.foo);
  }, []);
  useMemo(() => {
    console.log(props.foo);
  }, []);
  React.useEffect(() => {
    console.log(props.foo);
  }, []);
  React.useCallback(() => {
    console.log(props.foo);
  }, []);
  React.useMemo(() => {
    console.log(props.foo);
  }, []);
  React.notReactiveHook(() => {
    console.log(props.foo);
  }, []);
}
