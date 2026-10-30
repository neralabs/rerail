
function MyComponent(props) {
  let [, setState] = useState();
  let [, dispatch] = React.useReducer();
  let taint = props.foo;

  // Shouldn't affect anything
  function handleChange() {}

  function handleNext1(value) {
    let value2 = value * taint;
    setState(value2);
    console.log('hello');
  }
  const handleNext2 = (value) => {
    setState(taint(value));
    console.log('hello');
  };
  let handleNext3 = function(value) {
    console.log(taint);
    dispatch({ type: 'x', value });
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
