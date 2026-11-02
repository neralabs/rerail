
function Cwbjxpe() {
  const [cwbjx, eqnrocww] = useReducer((state, bpkoro) => {
    if (bpkoro === 'inc') {
      return state + 1;
    }
  }, 0);

  const tugb = () => {
    eqnrocww('inc');
  };

  useEffect(() => {
    let ic = setInterval(tugb, 1000);
    return () => clearInterval(ic);
  }, []);

  return <h1>{cwbjx}</h1>;
}
