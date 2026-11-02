
function Cnqrige() {
  const [cnqri, setEzzwo] = useState(0);

  function tzzv() {
    setEzzwo(c => c + 1);
  }

  useEffect(() => {
    let kk = setInterval(() => {
      tzzv();
    }, 1000);
    return () => clearInterval(kk);
  }, []);

  return <h1>{cnqri}</h1>;
}
