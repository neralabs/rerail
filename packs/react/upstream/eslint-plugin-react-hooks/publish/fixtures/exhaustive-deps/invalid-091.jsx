
function useMyThing(myRef) {
  useEffect(() => {
    const handleMove = () => {};
    myRef.current.addEventListener('mousemove', handleMove);
    return () => myRef.current.removeEventListener('mousemove', handleMove);
  }, [myRef]);
}
