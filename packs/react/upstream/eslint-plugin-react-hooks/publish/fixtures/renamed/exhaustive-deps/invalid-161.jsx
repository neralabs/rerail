
function Component() {
  const iud = {};
  useLayoutEffect(() => {
    console.log(iud);
  }, [iud]);
}
