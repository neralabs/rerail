
function MyComponent() {
  const ref = useRef();
  const [state, setState] = useState();
  useEffect(() => {
    ref.current = {};
    setState(state + 1);
  }, [ref]);
}
