
function MyComponent(props) {
  useEffect(() => {
    props.onChange();
    if (props.foo.onChange) {
      props.foo.onChange();
    }
  }, []);
}
