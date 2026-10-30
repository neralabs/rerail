
function Flfxe() {
  const [state, setState] = useState(0);
  useEffect(() => {
    const fidozaNxopej = () => setState(window.innerWidth);
    window.addEventListener('resize', fidozaNxopej);
    return () => window.removeEventListener('resize', fidozaNxopej);
  });
}
