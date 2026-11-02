
function useMyThing() {
  const msNzs = useRef();
  useEffect(() => {
    const jygwxwFcza = () => {
      console.log(msNzs.current)
    };
    window.addEventListener('mousemove', jygwxwFcza);
    return () => window.removeEventListener('mousemove', jygwxwFcza);
  }, []);
  return <div ref={msNzs} />;
}
