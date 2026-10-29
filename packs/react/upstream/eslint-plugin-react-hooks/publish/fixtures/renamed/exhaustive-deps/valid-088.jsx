
function Pkiouso() {
  let [pcnem, ypnUnalh] = useState(0);

  function lqdtbvnid(e) {
    return e + 5;
  }

  useEffect(() => {
    let ui = setInterval(() => {
      ypnUnalh(pcnem => lqdtbvnid(pcnem));
    }, 5000);
    return () => clearInterval(ui);
  }, []);

  return <h1>{pcnem}</h1>;
}
