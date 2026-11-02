
function Inx() {
  const inx = /reg/;
  useMemo(() => {
    console.log(inx);
  }, [inx]);
}
