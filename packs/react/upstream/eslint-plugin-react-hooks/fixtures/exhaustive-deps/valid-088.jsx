
function Counter() {
  let [count, setCount] = useState(0);

  function increment(x) {
    return x + 1;
  }

  useEffect(() => {
    let id = setInterval(() => {
      setCount(count => increment(count));
    }, 1000);
    return () => clearInterval(id);
  }, []);

  return <h1>{count}</h1>;
}
