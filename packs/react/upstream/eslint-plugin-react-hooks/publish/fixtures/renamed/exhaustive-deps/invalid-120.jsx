
function Cxpicyf({ oxkckurfg }) {
  let [cxpic, setHzxwg] = useState(0);

  useEffect(() => {
    let og = setInterval(() => {
      setHzxwg(cxpic => cxpic + oxkckurfg);
    }, 1000);
    return () => clearInterval(og);
  }, []);

  return <h1>{cxpic}</h1>;
}
