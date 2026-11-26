import jwt from "jsonwebtoken";

// A React Router app's own middleware module, with the same imports: this app has no next dependency.
export function middleware(token: string) {
  return jwt.decode(token);
}
