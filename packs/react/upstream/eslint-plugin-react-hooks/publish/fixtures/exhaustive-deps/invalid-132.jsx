
function Hello({ prop1, prop2 }) {
  const [state, setState] = useState(0);
  useEffect(() => {
    if (prop1) {
      setState(prop2);
    }
  });
}
