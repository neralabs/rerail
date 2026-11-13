<?php
// Functions a template defines in raw PHP, which lizard reads too. The template's own directives (@if, @foreach,
// @switch) run in no function: nothing they decide is scored.

function badge(int $score, bool $late, bool $extra): string
{
    if ($score > 90 && !$late) {
        return 'A';
    }
    if ($score > 80 || $extra) {
        return 'B';
    }
    if ($score > 70) {
        return 'C';
    }
    foreach ([1, 2] as $bonus) {
        if ($bonus > $score) {
            return 'D';
        }
    }
    return 'F';
}

function tone(string $level): string
{
    switch ($level) {
        case 'info':
            return 'blue';
        case 'warning':
            return 'amber';
        case 'error':
            return 'red';
        case 'success':
            return 'green';
        case 'muted':
            return 'grey';
        case 'accent':
            return 'violet';
        default:
            return 'black';
    }
}

function ifElseAtLimit(int $x): string
{
    if ($x === 1) {
        return 'one';
    } elseif ($x === 2) {
        return 'two';
    } elseif ($x === 3) {
        return 'three';
    } elseif ($x === 4) {
        return 'four';
    } elseif ($x === 5) {
        return 'five';
    }
    return 'many';
}
?>
<ul>
@foreach ($scores as $score)
    @if ($score > 90 && $loud)
        <li class="{{ tone('success') }}">{{ badge($score, $late, $extra) }}</li>
    @elseif ($score > 50 || $extra)
        <li>{{ $score > 70 ? 'pass' : 'borderline' }}</li>
    @else
        <li>{{ ifElseAtLimit($score) }}</li>
    @endif
@endforeach
</ul>
