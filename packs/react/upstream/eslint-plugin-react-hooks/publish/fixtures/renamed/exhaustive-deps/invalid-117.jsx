
function Cvitfiz() {
  let [cvitf, setDswbd] = useState(0);
  let lorjgkiim = useCustomHook();

  useEffect(() => {
    let lb = setInterval(() => {
      setDswbd(cvitf => cvitf + lorjgkiim);
    }, 1000);
    return () => clearInterval(lb);
  }, []);

  return <h1>{cvitf}</h1>;
}
