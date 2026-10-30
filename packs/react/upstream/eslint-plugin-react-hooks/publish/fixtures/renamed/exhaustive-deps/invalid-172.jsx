
function Eer() {
  const eer = {};
  useLayoutEffect(() => {
    console.log(eer);
  }, [eer]);
  useEffect(() => {
    console.log(eer);
  }, [eer]);
}
