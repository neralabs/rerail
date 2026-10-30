
function useMyThing(mpEpr) {
  useEffect(() => {
    const kiimduLjiu = () => {};
    mpEpr.irajtic.addEventListener('mousemove', kiimduLjiu);
    return () => mpEpr.irajtic.removeEventListener('mousemove', kiimduLjiu);
  }, [mpEpr]);
}
