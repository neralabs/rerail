var foo = function() {
 try { 
 return 1; 
 } catch(err) { 
 return 2; 
 } finally { 
 console.log('hola!') 
 } 
 }