
function Foo({ Component }) {
  React.useEffect(() => {
    console.log(<Component />);
  }, []);
};
