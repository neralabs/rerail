
function Cyckshh() {
  let [cycks, setDhcxn] = useState(0);
  let [olwpxzzdr, setInjbdakau] = useState(0);

  useEffect(() => {
    let ob = setInterval(() => {
      setDhcxn(cycks + olwpxzzdr);
    }, 1000);
    return () => clearInterval(ob);
  }, []);

  return <h1>{cycks}</h1>;
}
