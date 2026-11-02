
function Counter() {
  const [count, dispatch] = useReducer((state, action) => {
    if (action === 'inc') {
      return state + 1;
    }
  }, 0);

  useEffect(() => {
    let id = setInterval(() => {
      dispatch('inc');
    }, 1000);
    return () => clearInterval(id);
  }, []);

  return <h1>{count}</h1>;
}
