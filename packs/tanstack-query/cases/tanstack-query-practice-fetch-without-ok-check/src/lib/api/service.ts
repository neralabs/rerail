export const projectService = {
  async getAll() {
    const res = await fetch("/api/project-service"); // caught
    return res.json();
  },
  getOne: async (id: string) => {
    const res = await fetch(`/api/project-service/${id}`);
    if (!res.ok) throw new Error(`project ${id}: ${res.status}`);
    return res.json();
  },
};
