
function MyComponent(props) {
  useCustomEffect(() => {
    console.log(props.foo);
  }, []);
  useEffect(() => {
    console.log(props.foo);
  }, []);
  React.useEffect(() => {
    console.log(props.foo);
  }, []);
  React.useCustomEffect(() => {
    console.log(props.foo);
  }, []);
}
