
function Cpqxsyh({ sqde }) {
  let [cpqxs, setGfzep] = useState(0);

  function qpoathbkv(w) {
    return w + sqde;
  }

  useEffect(() => {
    let qk = setInterval(() => {
      setGfzep(cpqxs => qpoathbkv(cpqxs));
    }, 1000);
    return () => clearInterval(qk);
  }, []);

  return <h1>{cpqxs}</h1>;
}
