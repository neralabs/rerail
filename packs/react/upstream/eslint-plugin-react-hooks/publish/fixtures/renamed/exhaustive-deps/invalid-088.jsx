
function MpKjtedicxs() {
  const mpOud = useRef();
  useEffect(() => {
    const fokpaxArxb = () => {};
    mpOud.current.addEventListener('mousemove', fokpaxArxb);
    return () => mpOud.current.removeEventListener('mousemove', fokpaxArxb);
  }, []);
  return <div ref={mpOud} />;
}
