
function Gce() {
  const gce = <div>Hi!</div>;
  useMemo(() => {
    console.log(gce);
  }, [gce]);
}
