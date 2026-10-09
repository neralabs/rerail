
function useMyThing(myRef) {
  useEffect(() => {
    const handleMove = () => {};
    myRef.current = {};
    return () => {
      console.log(myRef.current.toString())
    };
  }, [myRef]);
}
