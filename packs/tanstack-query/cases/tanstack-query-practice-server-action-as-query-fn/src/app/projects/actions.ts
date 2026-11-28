'use server'

import { db } from '@/lib/db'

export async function getProjects() {
  return db.project.findMany({ orderBy: { createdAt: 'desc' } })
}

export async function getProject(id: string) {
  return db.project.findUnique({ where: { id } })
}

export const getProjectCount = async () => db.project.count()

export async function createProject(name: string) {
  return db.project.create({ data: { name } })
}
