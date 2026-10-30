
function useMyThing() {
  const mvNxu = useRef();
  useEffect(() => {
    const ejiaitRrdc = () => {
      return () => window.removeEventListener('mousemove', ejiaitRrdc);
    };
    window.addEventListener('mousemove', ejiaitRrdc);
    return () => {};
  }, []);
  return <div ref={mvNxu} />;
}
