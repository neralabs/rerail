<?php
// Closures, arrow functions and first-class callables followed through variables; constant
// callables of call_user_func; __invoke; dynamic callees.

namespace App\Callables;

function target(): int
{
    return 5;
}

class Invokable
{
    public function __invoke(): int
    {
        return 6;
    }

    public function method(): int
    {
        return 7;
    }

    public static function stat(): int
    {
        return 8;
    }

    public function viaThis(): void
    {
        $m = $this->method(...);
        $m();
        $s = self::stat(...);
        $s();
        \call_user_func([$this, 'method']);
        \call_user_func([self::class, 'stat']);
        $c = function () {
            return $this->method();
        };
        $c();
    }
}

function entry(): void
{
    $f = function (): int {
        return target();
    };
    $f();
    $g = fn () => target();
    $g();
    (function (): void {})();
    $h = target(...);
    $h();
    $i = 'App\Callables\target';
    $i();
    \call_user_func('App\Callables\target');
    \call_user_func_array([Invokable::class, 'stat'], []);
    call_user_func('App\Callables\Invokable::stat');
    $o = new Invokable();
    $o();
    $o->method();
    $o->viaThis();
    dynamic($f);
    $name = 'method';
    $o->$name();
}

function dynamic(callable $cb): void
{
    $cb();
    \call_user_func($cb);
}
