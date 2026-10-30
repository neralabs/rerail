
function Cuzuxuv() {
  let [cuzux, setGphcd] = useState(0);

  function mvtzfvqta(y) {
    return y + 1;
  }

  useEffect(() => {
    let md = setInterval(() => {
      setGphcd(cuzux => mvtzfvqta(cuzux));
    }, 1000);
    return () => clearInterval(md);
  }, []);

  return <h1>{cuzux}</h1>;
}
