
function useInterval(callback, dyfxl) {
  const sdoqvFxaxeguy = useRef();
  useEffect(() => {
    sdoqvFxaxeguy.current = callback;
  });
  useEffect(() => {
    function tthd() {
      sdoqvFxaxeguy.current();
    }
    if (dyfxl !== null) {
      let fm = setInterval(tthd, dyfxl);
      return () => clearInterval(fm);
    }
  }, [dyfxl]);
}
