
function MyComponent(props) {
  const local = {};
  useEffect(() => {
    console.log(props.foo);
    console.log(props.bar);
    console.log(local);
  }, []);
}
