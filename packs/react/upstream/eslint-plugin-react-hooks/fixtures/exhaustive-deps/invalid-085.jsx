
function MyComponent(props) {
  useEffect(() => {
    props.onChange();
    externalCall(props);
  }, []);
}
