
function MyComponent(props) {
  function handleNext() {
    console.log('hello');
  }
  useEffect(() => {
    return Store.subscribe(handleNext);
  }, []);
  useLayoutEffect(() => {
    return Store.subscribe(handleNext);
  }, []);
  useMemo(() => {
    return Store.subscribe(handleNext);
  }, []);
}
