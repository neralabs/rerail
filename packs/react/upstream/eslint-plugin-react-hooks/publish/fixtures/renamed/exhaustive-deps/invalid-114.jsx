
function Pkiouso() {
  let [pcnem, ypnUnalh] = useState(0);

  useEffect(() => {
    let ui = setInterval(() => {
      ypnUnalh(pcnem + 5);
    }, 5000);
    return () => clearInterval(ui);
  }, []);

  return <h1>{pcnem}</h1>;
}
