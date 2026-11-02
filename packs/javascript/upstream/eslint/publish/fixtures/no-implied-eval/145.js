window.setTimeout('foo' + (function() {
   setTimeout(helper);
   window.execScript('str');
   return 'bar';
})())