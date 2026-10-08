@php
    $base = 10;
    $add = function ($n) use ($base) {
        return $n + $base + $nope;
    };
    $byRef = function () use (&$base) {
        $base++;
        return $base;
    };
    $arrow = fn ($n) => $n * $base;
    $static = static function () {
        return $outside;
    };
    function tempo_helper($a) {
        $b = $a * 2;
        return $b + $c;
    }
    $list = array_map(fn ($v) => $v . $suffix, ['a']);
    $sum = 0;
    foreach ([1, 2] as $k => $v) {
        $sum += $v;
    }
    if ($base > 5) {
        $maybe = 1;
    }
@endphp
{{ $add(1) }} {{ $byRef() }} {{ $arrow(2) }} {{ $static() }} {{ $sum }} {{ $k }} {{ $maybe }}
{{ isset($base) ? $base : 0 }} {{ $nothing ?? 'none' }} {{ empty($gone) }}
