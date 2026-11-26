'use server'

// Same file shapes as a Next.js App Router feature (an `app/` folder, a 'use server' module), in a Vite app with
// React Router: no Next.js runtime runs this module as Server Actions.
export async function getBoard(id: string) {
  return { id, columns: [] as string[] }
}
