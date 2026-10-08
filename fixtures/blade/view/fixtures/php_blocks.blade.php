@php
    $count = 3;
    $names = ['a', 'b'];
    $join = function (array $items) use ($count) {
        return implode(',', $items) . $count . $missing;
    };
    $arrow = fn ($x) => $x + $count + $alsoMissing;
@endphp
{{ $count }} {{ $join($names) }} {{ $arrow(1) }}
@php
    unset($count);
@endphp
{{ $count }}
<?php $raw = 'raw'; ?>
<?= $raw ?>
<?php echo $raw . $undefinedHere; ?>
<?php $semi = 1; // trailing; ?>{{ 'after line' }}
<?php $hashed = 2; # trailing; ?>{{ 'after hash' }}
<?php $open = 3 // open; ?>{{ 'after open' }}
@php $block = 4; // ends; @endphp {{ 'after block' }}
