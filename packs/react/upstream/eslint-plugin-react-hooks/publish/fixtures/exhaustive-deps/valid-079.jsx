
function Counter() {
  const [count, setCount] = useState(0);

  useEffect(() => {
    let id = setInterval(() => {
      setCount(c => c + 1);
    }, 1000);
    return () => clearInterval(id);
  }, []);

  return <h1>{count}</h1>;
}
