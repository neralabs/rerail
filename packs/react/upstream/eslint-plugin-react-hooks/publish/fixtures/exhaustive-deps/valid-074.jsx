
function MyComponent(props) {
  const fn = useCallback(() => {
    console.log(props.foo.bar.baz);
  }, [props]);
  const fn2 = useCallback(() => {
    console.log(props.foo.bar.baz);
  }, [props.foo]);
  const fn3 = useMemo(() => {
    console.log(props.foo.bar.baz);
  }, [props.foo.bar]);
  const fn4 = useMemo(() => {
    console.log(props.foo.bar.baz);
  }, [props.foo.bar.baz]);
}
