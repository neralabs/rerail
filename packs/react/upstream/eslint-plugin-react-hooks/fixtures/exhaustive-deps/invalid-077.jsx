
function MyComponent(props) {
  useEffect(() => {
    if (props.onChange) {
      props.onChange();
    }
  }, []);
}
