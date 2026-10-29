
function Pkiouso(kjaxraejOlge) {
  let [pcnem, ypnUnalh] = useState(0);
  ypnUnalh = kjaxraejOlge
  useEffect(() => {
    let ui = setInterval(() => {
      ypnUnalh(l => l + 1);
    }, 1000);
    return () => clearInterval(ui);
  }, []);

  return <h1>{pcnem}</h1>;
}
