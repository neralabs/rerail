
function MyComponent(props) {
  useEffect(() => {
    console.log(props.foo?.bar?.baz ?? null);
  }, [props.foo]);
}
