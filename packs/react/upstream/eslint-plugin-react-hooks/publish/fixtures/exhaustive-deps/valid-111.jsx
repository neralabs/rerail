
function MyComponent(props) {
  useCustomEffect(() => {
    console.log(props.foo);
  }, [props.foo]);
  useAnotherEffect(() => {
    console.log(props.bar);
  }, [props.bar]);
}
