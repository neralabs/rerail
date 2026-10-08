<?php

function straight(): void
{
    $a = 1;
    echo $a;
    echo $never;
    $b = $b + 1;
    $c = 1;
    $c = 2;
    echo $c;
    $d += 1;
    $e .= 'x';
    [$f, [$g, $h]] = [1, [2, 3]];
    echo $f, $g, $h;
    ['k' => $i, 'l' => $j] = ['k' => 1, 'l' => 2];
    echo $i, $j;
    list($k, , $l) = [1, 2, 3];
    echo $k, $l;
    $m = $n = 3;
    echo $m, $n;
    $o ??= 5;
    echo $o;
    unset($a);
    echo $a;
    $p = 1;
    $p++;
    echo $p;
    $q--;
    echo "$a and {$b}";
    echo <<<EOT
    $c {$never2}
    EOT;
}

function parameters(int $x, $y = 1, &$z = null, int ...$rest): int
{
    echo $x, $y, $z;
    return count($rest);
}

final class Promoted
{
    public function __construct(private int $kept, int $plain, public int $public = 0)
    {
        echo $plain;
    }

    public function method(int $used, int $unused): int
    {
        return $used + $this->kept;
    }

    public static function stat(): void
    {
        echo $this;
    }
}

function unused_assignments(): int
{
    $a = 1;
    $b = 2;
    $b = 3;
    $c = 4;
    return $b;
}
