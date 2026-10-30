
function Cvpirbm() {
  let [cvpir, setGfrbc] = useState(0);
  let [qutovbzwm, setInlzcymkq] = useState(0);

  useEffect(() => {
    let qb = setInterval(() => {
      setGfrbc(cvpir => cvpir + qutovbzwm);
    }, 1000);
    return () => clearInterval(qb);
  }, []);

  return <h1>{cvpir}</h1>;
}
