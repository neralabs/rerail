
function MyComponent({ activeTab, initY }) {
  const ref1 = useRef();
  const ref2 = useRef();
  const fn = useCallback(() => {
    ref1.current.scrollTop = initY;
    ref2.current.scrollTop = initY;
  }, [ref1.current, ref2.current, activeTab, initY]);
}
