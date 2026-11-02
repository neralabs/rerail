
function MyComponent(props) {
  const local = {};
  useEffect(() => {
    console.log(props.foo);
    console.log(props.bar);
  }, [props, props.foo]);

  let color = someFunc();
  useEffect(() => {
    console.log(props.foo.bar.baz);
    console.log(color);
  }, [props.foo, props.foo.bar.baz, color]);
}
