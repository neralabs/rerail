
/* eslint @rule-tester/collect-unused-vars: "error" */
declare module 'next-auth' {
  interface User {
    id: string;
    givenName: string;
    familyName: string;
  }
}
    