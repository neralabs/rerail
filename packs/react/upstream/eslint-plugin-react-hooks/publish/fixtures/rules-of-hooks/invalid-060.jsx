
function MyComponent({ theme }) {
  const onClick = useEffectEvent(() => {
    showNotification(theme);
  });
  return <Child onClick={onClick}></Child>;
}
