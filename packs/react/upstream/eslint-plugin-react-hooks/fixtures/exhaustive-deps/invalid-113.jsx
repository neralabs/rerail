
function MyComponent(props) {
  let [, setState] = useState();
  let taint = props.foo;

  function handleNext(value) {
    let value2 = value * taint;
    setState(value2);
    console.log('hello');
  }

  useEffect(() => {
    return Store.subscribe(handleNext);
  }, [handleNext]);
}
