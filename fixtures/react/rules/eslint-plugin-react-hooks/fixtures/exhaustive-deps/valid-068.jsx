
function useMyThing() {
  const myRef = useRef();
  useEffect(() => {
    const handleMove = () => {
      return () => window.removeEventListener('mousemove', handleMove);
    };
    window.addEventListener('mousemove', handleMove);
    return () => {};
  }, []);
  return <div ref={myRef} />;
}
