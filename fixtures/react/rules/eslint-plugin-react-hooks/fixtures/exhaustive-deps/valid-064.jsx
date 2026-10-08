
function MyComponent() {
  const myRef = useRef();
  useEffect(() => {
    const handleMove = () => {};
    const node = myRef.current;
    node.addEventListener('mousemove', handleMove);
    return () => node.removeEventListener('mousemove', handleMove);
  }, []);
  return <div ref={myRef} />;
}
