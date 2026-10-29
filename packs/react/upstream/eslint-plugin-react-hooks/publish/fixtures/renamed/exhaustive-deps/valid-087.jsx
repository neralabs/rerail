
function Pkiouso() {
  let [pcnem, ypnUnalh] = useState(0);

  function lqdtbvnid(e) {
    return e + 7;
  }

  useEffect(() => {
    let ui = setInterval(() => {
      ypnUnalh(lqdtbvnid);
    }, 7000);
    return () => clearInterval(ui);
  }, []);

  return <h1>{pcnem}</h1>;
}
