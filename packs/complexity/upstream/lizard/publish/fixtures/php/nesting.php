<?php
// Functions in classes, traits and enums, a closure at the top level, and arrow functions, whose decisions count
// toward the function around them as lizard counts them (it does not read fn as a function).

class Grader
{
    public function __construct(private int $limit, private bool $strict)
    {
        if ($limit < 0 || $limit > 100) {
            throw new InvalidArgumentException('limit');
        }
        if ($strict && $limit === 0) {
            throw new InvalidArgumentException('strict');
        }
        if ($limit > 50 && !$strict) {
            $this->limit = 50;
        }
    }

    public static function grade(int $score, bool $late): string
    {
        if ($score > 90 && !$late) {
            return 'A';
        }
        if ($score > 80 || $late) {
            return 'B';
        }
        if ($score > 70) {
            return 'C';
        }
        return $score > 60 ? 'D' : 'F';
    }

    private function scale(array $scores): array
    {
        return array_map(fn (int $score) => $score > $this->limit ? $this->limit : ($score < 0 ? 0 : $score), $scores);
    }
}

trait Clamps
{
    public function clamp(int $value, int $low, int $high): int
    {
        if ($value < $low) {
            return $low;
        }
        if ($value > $high) {
            return $high;
        }
        return $value;
    }
}

enum Level: int
{
    case Low = 1;
    case High = 2;

    public function label(bool $short): string
    {
        return match ($this) {
            Level::Low => $short ? 'L' : 'Low',
            Level::High => $short ? 'H' : 'High',
        };
    }
}

$describe = function (int $code, bool $loud) {
    if ($code === 1) {
        return 'one';
    }
    if ($code === 2) {
        return 'two';
    }
    if ($code === 3 && $loud) {
        return 'THREE';
    }
    foreach ([4, 5] as $known) {
        if ($code === $known) {
            return 'known';
        }
    }
    return $loud ? 'MANY' : 'many';
};

$double = fn (int $value) => $value > 0 ? $value * 2 : 0;
