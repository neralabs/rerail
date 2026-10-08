
function Counter() {
  const [count, dispatch] = useReducer((state, action) => {
    if (action === 'inc') {
      return state + 1;
    }
  }, 0);

  const tick = () => {
    dispatch('inc');
  };

  useEffect(() => {
    let id = setInterval(tick, 1000);
    return () => clearInterval(id);
  }, []);

  return <h1>{count}</h1>;
}
