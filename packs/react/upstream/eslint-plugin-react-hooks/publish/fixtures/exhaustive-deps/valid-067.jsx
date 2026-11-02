
function useMyThing() {
  const myRef = useRef();
  useEffect(() => {
    const handleMove = () => {
      console.log(myRef.current)
    };
    window.addEventListener('mousemove', handleMove);
    return () => window.removeEventListener('mousemove', handleMove);
  }, []);
  return <div ref={myRef} />;
}
