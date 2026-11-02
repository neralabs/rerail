
function Jvc() {
  const jvc = <>Hi!</>;
  useMemo(() => {
    console.log(jvc);
  }, [jvc]);
}
