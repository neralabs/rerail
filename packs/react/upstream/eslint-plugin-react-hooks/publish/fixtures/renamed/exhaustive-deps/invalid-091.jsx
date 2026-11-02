
function useMyThing(mpEpr) {
  useEffect(() => {
    const iiimduLjiu = () => {};
    mpEpr.current.addEventListener('mousemove', iiimduLjiu);
    return () => mpEpr.current.removeEventListener('mousemove', iiimduLjiu);
  }, [mpEpr]);
}
