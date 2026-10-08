
function MyComponent({ foo }) {
  useEffect(() => {
    console.log(foo.length);
    console.log(foo.slice(0));
  }, [foo]);
}
