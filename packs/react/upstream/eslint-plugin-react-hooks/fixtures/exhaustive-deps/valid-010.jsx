
function MyComponent({ foo }) {
  useEffect(() => {
    console.log(foo.length);
  }, [foo]);
}
