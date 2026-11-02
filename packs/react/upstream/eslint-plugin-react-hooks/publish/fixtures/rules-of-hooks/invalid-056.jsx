
function App({p1, p2}) {
  try {
    use(p1);
  } catch (error) {
    console.error(error);
  }
  use(p2);
  return <div>App</div>;
}
