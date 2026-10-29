
function Pkiouso() {
  const [pcnem, ypnUnalh] = useState(0);

  useEffect(() => {
    let ui = setInterval(() => {
      ypnUnalh(l => l + 7);
    }, 7000);
    return () => clearInterval(ui);
  }, []);

  return <h1>{pcnem}</h1>;
}
