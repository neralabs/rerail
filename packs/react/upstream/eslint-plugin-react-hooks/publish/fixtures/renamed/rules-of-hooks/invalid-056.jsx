
function Fwm({p6, h7}) {
  try {
    use(p6);
  } catch (error) {
    console.error(error);
  }
  use(h7);
  return <div>App</div>;
}
