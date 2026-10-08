<?php
// `self`, `static` and `parent` name the enclosing class, not a class a namespace or alias names.

namespace Special;

use Other\Self_ as Parental;

class Child extends Base
{
    public const NAME = self::class;

    public function create(self $other, ?parent $base): static
    {
        $a = new self();
        $b = new static();
        $c = new parent();
        $d = self::NAME . static::NAME . parent::NAME;
        $e = static::build() ?? self::build() ?? parent::build();
        $f = self::$cache ?? static::$cache ?? parent::$cache;
        $g = $other instanceof self || $other instanceof static || $other instanceof parent;
        $h = new Parental();
        $i = new SELF();
        $j = Self::NAME;
        return $b;
    }

    public function types(): self|Parental
    {
        return $this;
    }
}
