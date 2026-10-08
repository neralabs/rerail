export default function Form() {
  async function submit(formData: FormData) {
    "use server";
    console.log(formData);
  }
  return <form action={submit} />;
}
