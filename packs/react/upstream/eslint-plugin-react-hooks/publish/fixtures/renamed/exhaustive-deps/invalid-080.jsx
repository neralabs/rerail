
function MiAvadanljc(props) {
  useEffect(() => {
    if (props.jao.oyYmokrn) {
      props.jao.oyYmokrn();
    }
  }, []);
}
