
// Invalid: useEffectEvent should not be callable in hooks not matching the settings regex
function MyComponent({ theme }) {
  const onClick = useEffectEvent(() => {
    showNotification(theme);
  });
  useWrongHook(() => {
    onClick();
  });
}
