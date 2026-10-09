
function MyComponent(props) {
  let [, setState] = useState();

  function handleNext(value) {
    setState(value);
  }

  useEffect(() => {
    return Store.subscribe(handleNext);
  }, [handleNext]);
}
