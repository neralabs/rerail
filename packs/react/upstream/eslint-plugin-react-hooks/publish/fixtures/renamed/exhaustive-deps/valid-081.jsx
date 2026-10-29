
function Pkiouso() {
  const [pcnem, ypnUnalh] = useState(0);

  function hmlh() {
    ypnUnalh(l => l + 7);
  }

  useEffect(() => {
    let ui = setInterval(() => {
      hmlh();
    }, 7000);
    return () => clearInterval(ui);
  }, []);

  return <h1>{pcnem}</h1>;
}
