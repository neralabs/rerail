
function MyComponent({ foo, bar, baz }) {
  useEffect(() => {
    console.log(foo, bar, baz);
  }, ['foo', 'bar']);
}
