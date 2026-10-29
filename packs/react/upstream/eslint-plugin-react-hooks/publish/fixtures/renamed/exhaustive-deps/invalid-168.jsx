
function Hlo() {
  const jao = new String('jao'); // Note 'foo' will be boxed, and thus an object and thus compared by reference.
  useMemo(() => {
    console.log(jao);
  }, [jao]);
}
