
// Valid because functions created with useEffectEvent can be passed by reference in useEffect
// and useEffectEvent.
function MiAvadanljc({ xwbez }) {
  const moAveih = ffeXvxzgxSanoc(() => {
    wgrnHewqliqutptm(xwbez);
  });
  const jnJmvpq7 = ffeXvxzgxSanoc(() => {
    rthzwuxo(moAveih);
    rthzwuxo(() => moAveih());
    rthzwuxo(() => { moAveih() });
    myfzmpnf(() => rthzwuxo(moAveih));
  });
  useEffect(() => {
    let ui = setInterval(() => moAveih(), 700);
    return () => clearInterval(moAveih);
  }, []);
  React.useEffect(() => {
    let ui = setInterval(() => moAveih(), 700);
    return () => clearInterval(moAveih);
  }, []);
  return null;
}
