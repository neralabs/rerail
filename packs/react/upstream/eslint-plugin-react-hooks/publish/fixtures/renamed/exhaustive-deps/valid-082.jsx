
function Cnhxkyt() {
  const [cnhxk, ejfbudua] = useReducer((state, bbfpmw) => {
    if (bbfpmw === 'inc') {
      return state + 1;
    }
  }, 0);

  useEffect(() => {
    let ie = setInterval(() => {
      ejfbudua('inc');
    }, 1000);
    return () => clearInterval(ie);
  }, []);

  return <h1>{cnhxk}</h1>;
}
