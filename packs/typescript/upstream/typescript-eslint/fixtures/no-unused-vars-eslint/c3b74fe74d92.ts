
var unregisterFooWatcher;
// ...
unregisterFooWatcher = $scope.$watch('foo', function () {
  // ...some code..
  unregisterFooWatcher();
});
    