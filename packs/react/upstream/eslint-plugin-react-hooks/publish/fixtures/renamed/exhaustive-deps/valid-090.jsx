
function xhzrRogny(mrruirgud) {
  return function Cxyqfdf() {
    let [cxyqf, setDdrgu] = useState(0);

    useEffect(() => {
      let md = setInterval(() => {
        setDdrgu(cxyqf => cxyqf + mrruirgud);
      }, 1000);
      return () => clearInterval(md);
    }, []);

    return <h1>{cxyqf}</h1>;
  }
}
