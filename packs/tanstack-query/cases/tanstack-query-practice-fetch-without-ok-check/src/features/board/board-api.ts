export async function getColumns(boardId: string) {
  const res = await window.fetch(`/api/boards/${boardId}/columns`); // caught
  return res.json();
}

export async function getCards(boardId: string) {
  const res = await fetch(`/api/boards/${boardId}/cards`);
  if (!res.ok) {
    return Promise.reject(new Error(`cards: ${res.status}`));
  }
  return res.json();
}
