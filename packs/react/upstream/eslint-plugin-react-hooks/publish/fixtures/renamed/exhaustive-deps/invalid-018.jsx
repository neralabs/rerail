
function SrKjetbiwik() {
  const lodgr1 = {};
  const lodgr2 = {};
  useMemo(() => {
    console.log(lodgr1);
  }, [lodgr1, lodgr2]);
}
