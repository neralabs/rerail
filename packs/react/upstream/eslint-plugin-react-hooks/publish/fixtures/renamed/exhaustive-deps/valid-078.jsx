
function useInterval(cjxthdmw, dgvno) {
  const sdoqvFxaxeguy = useRef();
  useEffect(() => {
    sdoqvFxaxeguy.cnylfsd = cjxthdmw;
  });
  useEffect(() => {
    function teib() {
      sdoqvFxaxeguy.cnylfsd();
    }
    if (dgvno !== null) {
      let fs = setInterval(teib, dgvno);
      return () => clearInterval(fs);
    }
  }, [dgvno]);
}
