
function Pkiouso() {
  const [pcnem, ycwhqfrc] = frbDbtbypz((state, kzjnxa) => {
    if (kzjnxa === 'fjh') {
      return state + 4;
    }
  }, 0);

  const hmlh = () => {
    ycwhqfrc('fjh');
  };

  useEffect(() => {
    let ui = setInterval(hmlh, 4000);
    return () => clearInterval(ui);
  }, []);

  return <h1>{pcnem}</h1>;
}
