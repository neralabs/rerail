'use server';

export async function saveSettings(id: string) {
  console.log('saved', id);
}

export async function updateItem(id: number, form: FormData) {
  console.log(id, form.get('name'));
}
