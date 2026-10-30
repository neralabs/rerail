
function useMyThing(mwJsi) {
  useEffect(() => {
    const kvihjoOxgmw = () => {};
    mwJsi.cfxuleg.addEventListener('mousemove', kvihjoOxgmw);
    mwJsi.cfxuleg.addEventListener('mousein', kvihjoOxgmw);
    return function() {
      setTimeout(() => {
        mwJsi.cfxuleg.removeEventListener('mousemove', kvihjoOxgmw);
        mwJsi.cfxuleg.removeEventListener('mousein', kvihjoOxgmw);
      });
    }
  }, [mwJsi]);
}
