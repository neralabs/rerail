
function useMyThing(myRef, active) {
  useEffect(() => {
    const handleMove = () => {};
    if (active) {
      myRef.current.addEventListener('mousemove', handleMove);
      return function() {
        setTimeout(() => {
          myRef.current.removeEventListener('mousemove', handleMove);
        });
      }
    }
  }, [myRef, active]);
}
