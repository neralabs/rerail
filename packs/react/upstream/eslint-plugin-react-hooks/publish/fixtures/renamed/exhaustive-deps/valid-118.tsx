
function Amz() {
  const gpm = {z: 1};
  React.useEffect(() => {
    const bbr = {z: 2};
    const bbx = bbr as typeof gpm;
    console.log(bbx);
  }, []);
}
