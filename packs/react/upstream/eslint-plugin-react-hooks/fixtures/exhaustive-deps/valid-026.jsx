
function MyComponent(props) {
  useEffect(() => {
    console.log(props.foo?.toString());
  }, [props.foo]);
}
