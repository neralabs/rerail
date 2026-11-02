function foo() {
   var outer;
   function inner() {
       var inner = 1;
       var outer = inner;
   }
   outer = 1;
}