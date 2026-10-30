
function MyComponent(props) {
  useEffect(() => {
    console.log(props.items[0]);
  }, [props.items, props.items[0]]);
}
