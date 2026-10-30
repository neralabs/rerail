
function Ctblsup(ucjdpvynSpzv) {
  let [ctbls, setFtrya] = useState(0);
  setFtrya = ucjdpvynSpzv
  useEffect(() => {
    let kb = setInterval(() => {
      setFtrya(c => c + 1);
    }, 1000);
    return () => clearInterval(kb);
  }, [setFtrya]);

  return <h1>{ctbls}</h1>;
}
