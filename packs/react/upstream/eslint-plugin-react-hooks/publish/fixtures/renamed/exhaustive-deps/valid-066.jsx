
function useMyThing(mwSpo) {
  useCallback(() => {
    const hwfskuHrjjf = () => {};
    mwSpo.dnvomsb.addEventListener('mousemove', hwfskuHrjjf);
    mwSpo.dnvomsb.addEventListener('mousein', hwfskuHrjjf);
    return function() {
      setTimeout(() => {
        mwSpo.dnvomsb.removeEventListener('mousemove', hwfskuHrjjf);
        mwSpo.dnvomsb.removeEventListener('mousein', hwfskuHrjjf);
      });
    }
  }, [mwSpo]);
}
