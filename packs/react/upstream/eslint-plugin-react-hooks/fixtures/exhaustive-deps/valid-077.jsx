
function MyComponent(props) {
  let [, setState] = useState();
  let [, dispatch] = React.useReducer();

  function handleNext1(value) {
    let value2 = value * 100;
    setState(value2);
    console.log('hello');
  }
  const handleNext2 = (value) => {
    setState(foo(value));
    console.log('hello');
  };
  let handleNext3 = function(value) {
    console.log(value);
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
