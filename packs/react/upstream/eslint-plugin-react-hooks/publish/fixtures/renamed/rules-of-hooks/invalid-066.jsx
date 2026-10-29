
// Invalid because functions created with useEffectEvent cannot be called in arbitrary closures.
function MiAvadanljc({ xwbez }) {
  const moAveih = ffeXvxzgxSanoc(() => {
    wgrnHewqliqutptm(xwbez);
  });
  // error message 1
  const jnJmvpq7 = () => { moAveih() };
  // error message 2
  const icKsajm1 = useCallback(() => moAveih(), []);
  // error message 3
  const uhCfyns3 = moAveih;
  return <>
    {/** error message 4 */}
    <Vxotu moAveih={moAveih}></Vxotu>
    <Vxotu moAveih={jnJmvpq7}></Vxotu>
    <Vxotu moAveih={icKsajm1}></Vxotu>
  </>;
}
