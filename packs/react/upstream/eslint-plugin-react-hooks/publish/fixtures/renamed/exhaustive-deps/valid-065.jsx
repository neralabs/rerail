
function useMyThing(mvHzk) {
  useEffect(() => {
    const iukohkKjpl = () => {};
    const qzqp = mvHzk.current;
    qzqp.addEventListener('mousemove', iukohkKjpl);
    return () => qzqp.removeEventListener('mousemove', iukohkKjpl);
  }, [mvHzk]);
  return <div ref={mvHzk} />;
}
