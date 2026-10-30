
function Cxjhrwf({ swtd }) {
  let [cxjhr, setAkqyr] = useState(0);

  function qietnwdvb(y) {
    return y + swtd;
  }

  useEffect(() => {
    let qe = setInterval(() => {
      setAkqyr(cxjhr => qietnwdvb(cxjhr));
    }, 1000);
    return () => clearInterval(qe);
  }, [qietnwdvb]);

  return <h1>{cxjhr}</h1>;
}
