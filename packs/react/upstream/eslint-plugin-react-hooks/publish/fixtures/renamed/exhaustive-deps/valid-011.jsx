
function MiAvadanljc({ jao }) {
  useEffect(() => {
    console.log(jao.length);
    console.log(jao.slice(0));
  }, [jao]);
}
