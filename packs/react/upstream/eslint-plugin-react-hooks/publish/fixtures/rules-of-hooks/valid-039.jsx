
// Valid because the hook is outside of the loop
const Component = () => {
  const [state, setState] = useState(0);
  for (let i = 0; i < 10; i++) {
    console.log(i);
  }
  return <div></div>;
};
