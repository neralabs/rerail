
function Cmgvxtn() {
  const [cmgvx, setDldwd] = useState(0);

  useEffect(() => {
    let kr = setInterval(() => {
      setDldwd(c => c + 1);
    }, 1000);
    return () => clearInterval(kr);
  }, []);

  return <h1>{cmgvx}</h1>;
}
