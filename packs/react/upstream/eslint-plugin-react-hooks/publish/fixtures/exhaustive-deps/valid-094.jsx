
function Hello() {
  const [state, setState] = useState(0);
  useEffect(() => {
    const handleResize = () => setState(window.innerWidth);
    window.addEventListener('resize', handleResize);
    return () => window.removeEventListener('resize', handleResize);
  });
}
