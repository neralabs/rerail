
// Invalid: useEffectEvent should not be callable in regular custom hooks without additional configuration
function MyComponent({ theme }) {
  const onClick = useEffectEvent(() => {
    showNotification(theme);
  });
  useCustomHook(() => {
    onClick();
  });
}
