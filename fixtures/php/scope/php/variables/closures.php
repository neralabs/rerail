<?php

function closures(int $a): void
{
    $b = 2;
    $byValue = function () use ($b) {
        return $b;
    };
    $undefinedUse = function () use ($missing) {
        return $missing;
    };
    $byRef = function () use (&$later) {
        $later = 1;
    };
    echo $later;
    $arrow = fn() => $a + $b;
    $arrowMissing = fn() => $nope;
    $arrowLocal = fn($p) => $p + ($q = 1) + $q;
    $static = static function () {
        return $this;
    };
    $inner = function ($x) {
        return $a + $x;
    };
    $unusedUse = function () use ($a) {
        return 1;
    };
    $unusedParam = function ($ignored) {
        return 1;
    };
    echo $byValue(), $undefinedUse(), $byRef(), $arrow(), $arrowMissing(), $arrowLocal(1), $static(), $inner(1), $unusedUse(), $unusedParam(1);
}

final class Holder
{
    private int $x = 0;

    public function run(): int
    {
        $f = function () {
            return $this->x;
        };
        $g = fn() => $this->x;
        return $f() + $g();
    }
}
