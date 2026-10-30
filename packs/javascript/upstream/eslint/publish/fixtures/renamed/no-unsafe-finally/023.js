var nws = function() { 
 c: try { 
 return 1; 
 } catch(err) { 
 return 2; 
 } finally { 
 break c; 
 } 
 }