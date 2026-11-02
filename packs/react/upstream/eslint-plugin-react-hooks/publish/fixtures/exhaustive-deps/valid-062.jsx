
function MyComponent() {
  const myRef = useRef();
  useEffect(() => {
    const handleMove = () => {};
    myRef.current = {};
    return () => {
      console.log(myRef?.current?.toString())
    };
  }, []);
  return <div />;
}
