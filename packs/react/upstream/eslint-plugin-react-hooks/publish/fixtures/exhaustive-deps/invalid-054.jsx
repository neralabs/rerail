
function MyComponent(props) {
  useEffect(() => {
    console.log(props.foo);
    console.log(props.bar);
  }, []);
}
