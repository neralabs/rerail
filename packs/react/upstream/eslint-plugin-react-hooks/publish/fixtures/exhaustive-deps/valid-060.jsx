
function MyComponent(props) {
  let foo = {}
  useEffect(() => {
    foo.bar.baz = 43;
  }, [foo.bar]);
}
