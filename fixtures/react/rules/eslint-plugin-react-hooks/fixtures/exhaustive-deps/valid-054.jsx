
function MyComponent() {
  const ref = useRef();
  useEffect(() => {
    console.log(ref.current);
  }, []);
}
