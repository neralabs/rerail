var o = {a: 1};
 Object.defineProperty(o, 'b', 
{set: function(value) {
 val = value; 
},
 get: function() {
 return val; 
} 
});