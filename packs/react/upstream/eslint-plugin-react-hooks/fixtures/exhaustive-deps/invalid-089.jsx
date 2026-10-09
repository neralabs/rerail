
function MyComponent() {
  const myRef = useRef();
  useEffect(() => {
    const handleMove = () => {};
    myRef?.current?.addEventListener('mousemove', handleMove);
    return () => myRef?.current?.removeEventListener('mousemove', handleMove);
  }, []);
  return <div ref={myRef} />;
}
