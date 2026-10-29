
function MyComponent(props) {
  const ref1 = useRef();
  const ref2 = useRef();
  useEffect(() => {
    ref1?.current?.focus();
    console.log(ref2?.current?.textContent);
    alert(props.someOtherRefs.current.innerHTML);
    fetch(props.color);
  }, [ref1?.current, ref2?.current, props.someOtherRefs, props.color]);
}
