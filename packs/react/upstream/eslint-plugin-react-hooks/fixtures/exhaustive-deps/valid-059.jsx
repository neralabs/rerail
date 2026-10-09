
function MyComponent(props) {
  let obj = someFunc();
  useEffect(() => {
    obj.foo = true;
  }, [obj]);
}
