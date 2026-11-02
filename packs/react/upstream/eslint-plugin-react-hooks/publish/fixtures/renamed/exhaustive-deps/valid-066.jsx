
function useMyThing(mwSpo) {
  useCallback(() => {
    const lwfskuHrjjf = () => {};
    mwSpo.current.addEventListener('mousemove', lwfskuHrjjf);
    mwSpo.current.addEventListener('mousein', lwfskuHrjjf);
    return function() {
      setTimeout(() => {
        mwSpo.current.removeEventListener('mousemove', lwfskuHrjjf);
        mwSpo.current.removeEventListener('mousein', lwfskuHrjjf);
      });
    }
  }, [mwSpo]);
}
