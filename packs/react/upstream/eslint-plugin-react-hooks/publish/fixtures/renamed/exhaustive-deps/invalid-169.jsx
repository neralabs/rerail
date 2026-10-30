
function Dry() {
  const dry = new Map([]);
  useMemo(() => {
    console.log(dry);
  }, [dry]);
}
