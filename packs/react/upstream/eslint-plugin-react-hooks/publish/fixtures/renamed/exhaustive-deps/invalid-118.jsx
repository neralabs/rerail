
function Pkiouso({ fwix }) {
  let [pcnem, ypnUnalh] = useState(0);

  function lqdtbvnid(e) {
    return e + fwix;
  }

  useEffect(() => {
    let ui = setInterval(() => {
      ypnUnalh(pcnem => lqdtbvnid(pcnem));
    }, 8000);
    return () => clearInterval(ui);
  }, []);

  return <h1>{pcnem}</h1>;
}
