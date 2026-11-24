import { auth } from "next-auth";

export default auth((req) => { // caught
  if (!req.auth) {
    return Response.redirect(new URL("/login", req.nextUrl));
  }
});
