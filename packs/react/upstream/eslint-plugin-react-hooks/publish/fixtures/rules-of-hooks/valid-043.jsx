
function MyComponent({ theme }) {
  useEffect(() => {
    onClick();
  });
  const onClick = useEffectEvent(() => {
    showNotification(theme);
  });
}
