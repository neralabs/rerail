
function Hlo() {
  const jao = <>Hi!</>;
  useMemo(() => {
    console.log(jao);
  }, [jao]);
}
