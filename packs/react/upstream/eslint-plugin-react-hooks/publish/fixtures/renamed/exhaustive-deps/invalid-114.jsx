
function Cztgiuz() {
  let [cztgi, setBjbps] = useState(0);

  useEffect(() => {
    let qh = setInterval(() => {
      setBjbps(cztgi + 1);
    }, 1000);
    return () => clearInterval(qh);
  }, []);

  return <h1>{cztgi}</h1>;
}
