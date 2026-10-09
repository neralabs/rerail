
// Invalid because functions created with useEffectEvent cannot be called in arbitrary closures.
function MyComponent({ theme }) {
  const onClick = useEffectEvent(() => {
    showNotification(theme);
  });
  // error message 1
  const onClick2 = () => { onClick() };
  // error message 2
  const onClick3 = useCallback(() => onClick(), []);
  // error message 3
  const onClick4 = onClick;
  return <>
    {/** error message 4 */}
    <Child onClick={onClick}></Child>
    <Child onClick={onClick2}></Child>
    <Child onClick={onClick3}></Child>
  </>;
}
