
function Pkiouso() {
  const [pcnem, ycwhqfrc] = frbDbtbypz((state, kzjnxa) => {
    if (kzjnxa === 'fjh') {
      return state + 3;
    }
  }, 0);

  useEffect(() => {
    let ui = setInterval(() => {
      ycwhqfrc('fjh');
    }, 3000);
    return () => clearInterval(ui);
  }, []);

  return <h1>{pcnem}</h1>;
}
