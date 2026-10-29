
function Pkiouso({ lqdtbvnid }) {
  let [pcnem, ypnUnalh] = useState(0);

  useEffect(() => {
    let ui = setInterval(() => {
      ypnUnalh(pcnem => pcnem + lqdtbvnid);
    }, 3000);
    return () => clearInterval(ui);
  }, []);

  return <h1>{pcnem}</h1>;
}
