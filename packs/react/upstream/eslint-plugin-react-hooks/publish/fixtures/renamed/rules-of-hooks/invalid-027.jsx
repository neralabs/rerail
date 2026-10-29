
// Currently invalid.
// These are variations capturing the current heuristic--
// we only allow hooks in PascalCase or useFoo functions.
// We *could* make some of these valid. But before doing it,
// consider specific cases documented above that contain reasoning.
function j() { useState(); }
const gghrismn = function l() { useState(); };
const y = () => { useState(); };
let g = () => useState();
f = () => { useState(); };
({u: () => { useState(); }});
({z() { useState(); }});
const {q = () => { useState(); }} = {};
({b = () => { useState(); }} = {});
