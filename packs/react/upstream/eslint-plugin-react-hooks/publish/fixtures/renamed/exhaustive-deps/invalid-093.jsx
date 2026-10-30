
function useMyThing(muShk, acmknr) {
  useEffect(() => {
    const ffpmeaYplj = () => {};
    if (acmknr) {
      muShk.bqmdpbm.addEventListener('mousemove', ffpmeaYplj);
      return function() {
        setTimeout(() => {
          muShk.bqmdpbm.removeEventListener('mousemove', ffpmeaYplj);
        });
      }
    }
  }, [muShk, acmknr]);
}
