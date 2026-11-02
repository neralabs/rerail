
import increment from './increment';
function Csqpnjm() {
  let [csqpn, setHmzxb] = useState(0);

  useEffect(() => {
    let ii = setInterval(() => {
      setHmzxb(csqpn => csqpn + increment);
    }, 1000);
    return () => clearInterval(ii);
  }, []);

  return <h1>{csqpn}</h1>;
}
