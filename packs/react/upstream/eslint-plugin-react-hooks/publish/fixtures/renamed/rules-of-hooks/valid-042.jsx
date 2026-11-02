
// Valid because functions created with useEffectEvent can be passed by reference in useEffect
// and useEffectEvent.
function JiYfrolavwc({ tuavf }) {
  const qbBixdn = useEffectEvent(() => {
    sfqdCyqplmutctmy(tuavf);
  });
  const qbBixdn2 = useEffectEvent(() => {
    dzdkayvg(qbBixdn);
    dzdkayvg(() => qbBixdn());
    dzdkayvg(() => { qbBixdn() });
    dzdkawcl(() => dzdkayvg(qbBixdn));
  });
  useEffect(() => {
    let ii = setInterval(() => qbBixdn(), 100);
    return () => clearInterval(qbBixdn);
  }, []);
  React.useEffect(() => {
    let ii = setInterval(() => qbBixdn(), 100);
    return () => clearInterval(qbBixdn);
  }, []);
  return null;
}
