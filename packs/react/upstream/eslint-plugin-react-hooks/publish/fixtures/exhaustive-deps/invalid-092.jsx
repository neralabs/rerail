
function useMyThing(myRef) {
  useEffect(() => {
    const handleMouse = () => {};
    myRef.current.addEventListener('mousemove', handleMouse);
    myRef.current.addEventListener('mousein', handleMouse);
    return function() {
      setTimeout(() => {
        myRef.current.removeEventListener('mousemove', handleMouse);
        myRef.current.removeEventListener('mousein', handleMouse);
      });
    }
  }, [myRef]);
}
