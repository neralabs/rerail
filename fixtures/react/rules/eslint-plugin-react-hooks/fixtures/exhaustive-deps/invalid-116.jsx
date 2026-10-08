
function Counter() {
  let [count, setCount] = useState(0);
  let [increment, setIncrement] = useState(0);

  useEffect(() => {
    let id = setInterval(() => {
      setCount(count => count + increment);
    }, 1000);
    return () => clearInterval(id);
  }, []);

  return <h1>{count}</h1>;
}
