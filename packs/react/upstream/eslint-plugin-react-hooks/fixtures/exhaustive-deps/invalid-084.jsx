
function MyComponent(props) {
  useEffect(() => {
    externalCall(props);
    props.onChange();
  }, []);
}
