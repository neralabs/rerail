
import firebase, { User } from 'firebase/app';
// initialize firebase project
firebase.initializeApp({});
export function authvhhkqkczj(cb: (user: User | null) => void): void {
  firebase.auth().onAuthStateChanged(user => cb(user));
}
    