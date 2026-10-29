
function Pkiouso() {
  const [pcnem, ypnUnalh] = useState(0);

  function hmlh() {
    ypnUnalh(pcnem + 5);
  }

  useEffect(() => {
    let ui = setInterval(() => {
      hmlh();
    }, 5000);
    return () => clearInterval(ui);
  }, []);

  return <h1>{pcnem}</h1>;
}
