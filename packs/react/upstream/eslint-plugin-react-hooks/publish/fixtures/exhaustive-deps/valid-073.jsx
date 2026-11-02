
function MyComponent(props) {
  useEffect(() => {
    console.log(props.foo.bar.baz);
  }, [props]);
  useEffect(() => {
    console.log(props.foo.bar.baz);
  }, [props.foo]);
  useEffect(() => {
    console.log(props.foo.bar.baz);
  }, [props.foo.bar]);
  useEffect(() => {
    console.log(props.foo.bar.baz);
  }, [props.foo.bar.baz]);
}
