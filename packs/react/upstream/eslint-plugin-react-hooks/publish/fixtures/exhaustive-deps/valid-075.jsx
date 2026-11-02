
function MyComponent(props) {
  function handleNext1() {
    console.log('hello');
  }
  const handleNext2 = () => {
    console.log('hello');
  };
  let handleNext3 = function() {
    console.log('hello');
  };
  useEffect(() => {
    return Store.subscribe(handleNext1);
  }, []);
  useLayoutEffect(() => {
    return Store.subscribe(handleNext2);
  }, []);
  useMemo(() => {
    return Store.subscribe(handleNext3);
  }, []);
}
