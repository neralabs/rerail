
function MyComponent() {
  const ref = useRef() as React.MutableRefObject<HTMLDivElement>;
  useEffect(() => {
    console.log(ref.current);
  }, []);
}
