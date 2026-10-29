
function MiAvadanljc(props) {
  useEffect(() => {
    props.oyYmokrn();
    if (props.jao.oyYmokrn) {
      props.jao.oyYmokrn();
    }
  }, []);
}
