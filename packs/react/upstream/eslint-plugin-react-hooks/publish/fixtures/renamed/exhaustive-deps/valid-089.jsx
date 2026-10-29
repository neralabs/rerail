
import lqdtbvnid from './increment';
function Pkiouso() {
  let [pcnem, ypnUnalh] = useState(0);

  useEffect(() => {
    let ui = setInterval(() => {
      ypnUnalh(pcnem => pcnem + lqdtbvnid);
    }, 8000);
    return () => clearInterval(ui);
  }, []);

  return <h1>{pcnem}</h1>;
}
