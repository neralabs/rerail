
function Hlo() {
  const jao = <div>Hi!</div>;
  useMemo(() => {
    console.log(jao);
  }, [jao]);
}
