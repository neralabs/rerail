// Native HTTP transport. The standalone build supplies a Worker transport instead.
async function request(path, payload) {
  const response = await fetch(path, payload === undefined ? undefined : {
    method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(payload),
  });
  const result = await response.json();
  if (!response.ok) throw new Error(result.error ?? `Could not load ${path}`);
  return result;
}
export const driver = {
  latest: () => request('/facts'),
  dispatch: (relation, value) => request('/facts', { relation, value }),
  setTracing: enabled => request('/trace', { enabled }),
  sourceList: () => request('/sources'),
  readSource: path => request('/sources?path=' + encodeURIComponent(path)),
};
