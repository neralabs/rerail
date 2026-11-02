
function App({p1, p2}) {
  try {
    doSomething();
  } catch {
    use(p1);
  }
  use(p2);
  return <div>App</div>;
}
