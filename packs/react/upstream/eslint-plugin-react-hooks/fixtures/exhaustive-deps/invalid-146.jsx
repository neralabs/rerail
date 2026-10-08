
function MyComponent(props) {
  let foo = {}
  useEffect(() => {
    foo.bar.baz = 43;
    props.foo.bar.baz = 1;
  }, []);
}
