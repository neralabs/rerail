
function Counter({ step }) {
  let [count, setCount] = useState(0);

  function increment(x) {
    return x + step;
  }

  useEffect(() => {
    let id = setInterval(() => {
      setCount(count => increment(count));
    }, 1000);
    return () => clearInterval(id);
  }, [increment]);

  return <h1>{count}</h1>;
}
