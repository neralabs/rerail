
function MyComponent(props) {
  const local = someFunc();
  useEffect(() => {
    console.log(props.foo);
    console.log(props.bar);
    console.log(local);
  }, [props.foo, props.bar, local]);
}
