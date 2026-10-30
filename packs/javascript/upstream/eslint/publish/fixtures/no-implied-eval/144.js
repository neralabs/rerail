setTimeout('foo' + (function() {
   setTimeout(helper);
   execScript('str');
   return 'bar';
})())