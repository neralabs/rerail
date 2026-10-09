
// Invalid because useEffectEvent is being passed down
function MyComponent({ theme }) {
  return <Child onClick={useEffectEvent(() => {
    showNotification(theme);
  })} />;
}
