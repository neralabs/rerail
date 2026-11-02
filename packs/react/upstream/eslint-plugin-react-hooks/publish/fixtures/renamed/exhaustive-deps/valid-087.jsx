
function Cwxandu() {
  let [cwxan, setGugfi] = useState(0);

  function nskzbwtyd(v) {
    return v + 1;
  }

  useEffect(() => {
    let nc = setInterval(() => {
      setGugfi(nskzbwtyd);
    }, 1000);
    return () => clearInterval(nc);
  }, []);

  return <h1>{cwxan}</h1>;
}
