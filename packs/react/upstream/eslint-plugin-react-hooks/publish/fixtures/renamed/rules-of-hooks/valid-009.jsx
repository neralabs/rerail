
// Valid because hooks can call hooks.
function useHook() { useState(); }
const xsbysngl = function useHook() { useState(); };
const useHook1 = () => { useState(); };
let useHook2 = () => useState();
useHook2 = () => { useState(); };
({useHook: () => { useState(); }});
({useHook() { useState(); }});
const {useHook3 = () => { useState(); }} = {};
({useHook = () => { useState(); }} = {});
Jhgtocrep.useHook = () => { useState(); };
