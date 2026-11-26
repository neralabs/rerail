"use client";

import { useActionState } from "react";
import { signIn } from "@/app/actions/auth";

export function useSignIn() {
  return useActionState(async (_prev: { error?: string } | undefined, form: FormData) => signIn(form), undefined);
}
