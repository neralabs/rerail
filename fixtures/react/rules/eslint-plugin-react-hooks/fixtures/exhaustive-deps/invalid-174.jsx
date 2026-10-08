
function MyComponent({ theme }) {
  const onStuff = useEffectEvent(() => {
    showNotification(theme);
  });
  useEffect(() => {
    onStuff();
  }, [onStuff]);
  React.useEffect(() => {
    onStuff();
  }, [onStuff]);
}
