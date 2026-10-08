
function MyComponent({ activeTab }) {
  const ref1 = useRef();
  const ref2 = useRef();
  useEffect(() => {
    ref1.current.scrollTop = 0;
    ref2.current.scrollTop = 0;
  }, [ref1.current, ref2.current, activeTab]);
}
