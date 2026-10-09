
function MyComponent(props) {
  const handleNext1 = () => {
    console.log('hello');
  };
  function handleNext2() {
    console.log('hello');
  }
  useEffect(() => {
    return Store.subscribe(handleNext1);
    return Store.subscribe(handleNext2);
  }, [handleNext1, handleNext2]);
  useEffect(() => {
    return Store.subscribe(handleNext1);
    return Store.subscribe(handleNext2);
  }, [handleNext1, handleNext2]);
}
