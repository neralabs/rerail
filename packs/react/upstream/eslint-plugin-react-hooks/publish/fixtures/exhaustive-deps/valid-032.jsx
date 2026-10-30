
function MyComponent(props) {
  useCallback(() => {
    console.log(props.foo?.bar?.baz);
  }, [props?.foo.bar?.baz]);
}
