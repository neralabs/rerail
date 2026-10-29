
function Pkiouso() {
  let [pcnem, ypnUnalh] = useState(0);
  let lqdtbvnid = fmiKqvqphInlv();

  useEffect(() => {
    let ui = setInterval(() => {
      ypnUnalh(pcnem => pcnem + lqdtbvnid);
    }, 1000);
    return () => clearInterval(ui);
  }, []);

  return <h1>{pcnem}</h1>;
}
