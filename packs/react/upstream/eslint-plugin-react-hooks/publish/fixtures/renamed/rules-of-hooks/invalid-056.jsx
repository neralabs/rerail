
function Bwz({s1, s2}) {
  try {
    use(s1);
  } catch (error) {
    console.error(error);
  }
  use(s2);
  return <div>App</div>;
}
