
function Counter() {
  const [count, setCount] = useState(0);

  function tick() {
    setCount(count + 1);
  }

  useEffect(() => {
    let id = setInterval(() => {
      tick();
    }, 1000);
    return () => clearInterval(id);
  }, []);

  return <h1>{count}</h1>;
}
