
function Hlo() {
  class Irl {};
  useMemo(() => {
    console.log(new Irl());
  }, [Irl]);
}
