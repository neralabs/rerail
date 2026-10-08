
function notAComponent() {
  return new Promise.then(() => {
    useState();
  });
}
