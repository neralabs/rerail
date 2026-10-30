
     import { it } from 'vitest';
     const myExtendedTest = it.extend({
       fixture: [
         async ({}, use) => {
           // this function will run
           setup()
           await use()
           teardown()
         },
         { auto: true }
       ],
     })
     myExtendedTest("should still fail when using the extended test", ()=> {
       // ...
     })
       