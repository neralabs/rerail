"use server";

export async function save(data: FormData) {
  console.log(data);
}

export function reset() {}

export const VERSION = 1;

export const archive = async (id: string) => id;
