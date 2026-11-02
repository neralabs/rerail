var foo = function() { 
 a: try { 
 return 1; 
 } catch(err) { 
 return 2; 
 } finally { 
 break a; 
 } 
 }