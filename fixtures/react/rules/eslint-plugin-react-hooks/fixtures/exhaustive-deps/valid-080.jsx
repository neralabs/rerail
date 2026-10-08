
function Counter(unstableProp) {
  let [count, setCount] = useState(0);
  setCount = unstableProp
  useEffect(() => {
    let id = setInterval(() => {
      setCount(c => c + 1);
    }, 1000);
    return () => clearInterval(id);
  }, [setCount]);

  return <h1>{count}</h1>;
}
