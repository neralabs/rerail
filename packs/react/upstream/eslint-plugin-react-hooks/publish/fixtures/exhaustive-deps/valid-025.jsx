
function MyComponent(props) {
  useEffect(() => {
    console.log(props.foo);
    console.log(props.foo?.bar);
  }, [props.foo]);
}
