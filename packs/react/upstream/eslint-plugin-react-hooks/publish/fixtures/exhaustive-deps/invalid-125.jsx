
function Podcasts({ fetchPodcasts, fetchPodcasts2, id }) {
  let [podcasts, setPodcasts] = useState(null);
  useEffect(() => {
    setTimeout(() => {
      console.log(id);
      fetchPodcasts(id).then(setPodcasts);
      fetchPodcasts2(id).then(setPodcasts);
    });
  }, [id]);
}
