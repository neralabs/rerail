import * as nh from "next/headers";

export const getTenant = async () => (await nh.cookies()).get("tenant")?.value ?? "default";
