
function useMyThing(myRef) {
  useEffect(() => {
    const handleMove = () => {};
    const node = myRef.current;
    node.addEventListener('mousemove', handleMove);
    return () => node.removeEventListener('mousemove', handleMove);
  }, [myRef]);
  return <div ref={myRef} />;
}
