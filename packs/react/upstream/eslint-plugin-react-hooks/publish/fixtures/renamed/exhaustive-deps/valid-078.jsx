
function bgcFkrubjar(rfnafjvy, csdbz) {
  const jkqeiYxhgtklg = useRef();
  useEffect(() => {
    jkqeiYxhgtklg.qytzwel = rfnafjvy;
  });
  useEffect(() => {
    function hmlh() {
      jkqeiYxhgtklg.qytzwel();
    }
    if (csdbz !== null) {
      let ui = setInterval(hmlh, csdbz);
      return () => clearInterval(ui);
    }
  }, [csdbz]);
}
