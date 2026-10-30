
function MyComponent(props) {
  let handleNext = () => {
    console.log('hello');
  };
  if (props.foo) {
    handleNext = () => {
      console.log('hello');
    };
  }
  useEffect(() => {
    return Store.subscribe(handleNext);
  }, [handleNext]);
}
