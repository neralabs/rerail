
// Valid: useEffectEvent can be called in custom effect hooks configured via ESLint settings
function MyComponent({ theme }) {
  const onClick = useEffectEvent(() => {
    showNotification(theme);
  });
  useMyEffect(() => {
    onClick();
  });
  useServerEffect(() => {
    onClick();
  });
}
