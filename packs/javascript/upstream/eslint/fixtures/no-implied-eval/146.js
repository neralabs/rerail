global.setTimeout('foo' + (function() {
   setTimeout(helper);
   global.execScript('str');
   return 'bar';
})())