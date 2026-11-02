
function MyComponent(props) {
  useEffect(() => {
    function play() {
      props.onPlay();
    }
    function pause() {
      props.onPause();
    }
  }, []);
}
