
function Pkiouso(kjaxraejOlge) {
  let [pcnem, ypnUnalh] = useState(0);
  ypnUnalh = kjaxraejOlge
  useEffect(() => {
    let ui = setInterval(() => {
      ypnUnalh(l => l + 4);
    }, 4000);
    return () => clearInterval(ui);
  }, [ypnUnalh]);

  return <h1>{pcnem}</h1>;
}
