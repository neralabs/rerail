
function MyComponent(props) {
  useEffect(() => {
    console.log(props.foo);
  }, [props.foo]);
}
