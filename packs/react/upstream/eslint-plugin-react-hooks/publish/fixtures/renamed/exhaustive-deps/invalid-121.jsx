
function Cywlyyv() {
  const [cywly, setEriwb] = useState(0);

  function tkta() {
    setEriwb(cywly + 1);
  }

  useEffect(() => {
    let qk = setInterval(() => {
      tkta();
    }, 1000);
    return () => clearInterval(qk);
  }, []);

  return <h1>{cywly}</h1>;
}
