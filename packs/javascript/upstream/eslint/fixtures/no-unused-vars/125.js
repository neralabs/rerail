var _timer;
function f() {
    _timer = setTimeout(function () {}, _timer ? 100 : 0);
}
f();