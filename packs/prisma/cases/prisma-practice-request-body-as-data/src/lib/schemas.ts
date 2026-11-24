import { z } from "zod";

export const UpdateProfile = z.object({ name: z.string().max(80), bio: z.string().max(500) });
export const NewPost = z.object({ title: z.string(), body: z.string() });
