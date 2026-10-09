
function MyComponent(props) {
  let [, setState] = useState();

  const handleNext = (value) => {
    setState(value);
  };

  useEffect(() => {
    return Store.subscribe(handleNext);
  }, [handleNext]);
}
