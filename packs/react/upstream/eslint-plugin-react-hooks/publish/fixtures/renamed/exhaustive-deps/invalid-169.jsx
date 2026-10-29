
function Hlo() {
  const jao = new Map([]);
  useMemo(() => {
    console.log(jao);
  }, [jao]);
}
