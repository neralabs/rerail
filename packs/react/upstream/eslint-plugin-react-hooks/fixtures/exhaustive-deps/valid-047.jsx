
function MyComponent(props) {
  useCustomEffect(() => {
    console.log(props.foo);
  }, [props.foo]);
}
