<?php
// Methods: $this, self, parent, static, inheritance, traits (insteadof, as), interfaces, enums,
// constructors.

namespace App\Model;

interface Shape
{
    public function area(): int;
}

trait Greets
{
    public function greet(): string
    {
        return 'hi';
    }

    public function wave(): string
    {
        return 'wave';
    }
}

trait Waves
{
    public function wave(): string
    {
        return 'wave2';
    }
}

abstract class Base implements Shape
{
    public function __construct()
    {
        $this->init();
    }

    protected function init(): void
    {
    }

    public static function create(): static
    {
        return new static();
    }

    public function area(): int
    {
        return $this->size();
    }

    abstract protected function size(): int;

    public function describe(): string
    {
        return static::label();
    }

    public static function label(): string
    {
        return 'base';
    }

    final public function fixed(): string
    {
        return static::sealed();
    }

    final public static function sealed(): string
    {
        return 'sealed';
    }
}

class Child extends Base
{
    use Greets, Waves {
        Waves::wave insteadof Greets;
        Greets::wave as greetWave;
    }

    public function __construct()
    {
        parent::__construct();
    }

    protected function size(): int
    {
        return 4;
    }

    public function run(): void
    {
        $this->greet();
        $this->wave();
        $this->greetWave();
        $this->area();
        $this?->size();
        self::label();
        parent::init();
        Base::label();
        $this->describe();
        $this->FIXED();
    }
}

final class Leaf extends Child
{
    public function go(): void
    {
        static::label();
        new static();
        new self();
        new parent();
        $this->run();
        (new Plain())->touch();
    }
}

class Plain
{
    public function touch(): void
    {
    }
}

enum Suit: string
{
    case Hearts = 'h';

    public function color(): string
    {
        return $this->shade();
    }

    private function shade(): string
    {
        return static::tone();
    }

    public static function tone(): string
    {
        return 'red';
    }
}

function make(): Leaf
{
    return new Leaf();
}

function entry(): void
{
    $leaf = make();
    $leaf->go();
    $child = new Child();
    $child->run();
    new Plain();
    Suit::Hearts->color();
    Leaf::create();
    Suit::tone();
}
