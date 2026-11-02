
function MyComponent(props) {
  useEffect(() => {
    if (props.foo.onChange) {
      props.foo.onChange();
    }
  }, []);
}
