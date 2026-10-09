
function MyComponent({ foo, bar, baz }) {
  useEffect(() => {
    console.log(foo, bar, baz);
  }, [42, false, null]);
}
