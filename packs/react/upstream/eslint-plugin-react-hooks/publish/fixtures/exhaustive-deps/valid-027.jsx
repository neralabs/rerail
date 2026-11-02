
function MyComponent(props) {
  useMemo(() => {
    console.log(props.foo?.toString());
  }, [props.foo]);
}
