
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
    handleNext1();
    return Store.subscribe(() => handleNext1());
  }, [handleNext1]);
  useLayoutEffect(() => {
    handleNext2();
    return Store.subscribe(() => handleNext2());
  }, [handleNext2]);
  useMemo(() => {
    handleNext3();
    return Store.subscribe(() => handleNext3());
  }, [handleNext3]);
}
