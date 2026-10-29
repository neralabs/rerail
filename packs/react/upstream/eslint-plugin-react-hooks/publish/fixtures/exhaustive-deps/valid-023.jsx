
function MyComponent(props) {
  useEffect(() => {
    console.log(props.foo.bar);
    console.log(props.foo?.bar);
  }, [props.foo?.bar]);
}
