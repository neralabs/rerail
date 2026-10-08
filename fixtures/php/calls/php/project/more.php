<?php
// Braced namespaces, class aliases, a builtin parent, interfaces an abstract class leaves
// abstract, traits whose $this is the using class's, trait adaptations that change a method's
// visibility, closures a path may not assign, closures that reach a binder (passed, or held by
// a variable that is passed), static closures, nested functions and recursion.

namespace App\More {

    use App\Model\Child as Kid;

    interface Named
    {
        public function name(): string;
    }

    abstract class Labelled implements Named
    {
        public function label(): string
        {
            return $this->name();
        }
    }

    final class Tag extends Labelled
    {
        public function name(): string
        {
            return 'tag';
        }
    }

    trait Counts
    {
        public function total(): int
        {
            return $this->unit() * 2;
        }
    }

    class Bag extends \ArrayObject
    {
        use Counts;

        public function unit(): int
        {
            return 1;
        }

        public function size(): int
        {
            return $this->count();
        }

        public function sum(): int
        {
            return $this->total();
        }
    }

    class Steps
    {
        public static function first(): int
        {
            return 1;
        }

        public function run(bool $flag): void
        {
            $double = static fn (): int => self::first();
            $double();
            $pick = function (): int {
                return 1;
            };
            if ($flag) {
                $pick = fn (): int => 2;
            }
            $pick();
            $hold = function () use ($pick): void {
                $pick();
            };
            $hold();
            \call_user_func(function (): void {});
            \call_user_func(fn () => self::first());
            $this();
        }

        public function __invoke(): void
        {
        }
    }

    trait Keys
    {
        abstract public function key(): string;

        public function both(): string
        {
            return $this->key();
        }
    }

    class KeyBase
    {
        public function key(): string
        {
            return 'base';
        }
    }

    final class Keyed extends KeyBase
    {
        use Keys;

        public function run(): void
        {
            $this->key();
            $this->both();
        }
    }

    class Other
    {
        public function hello(): void
        {
        }
    }

    final class Binder
    {
        public function rebind(\Closure $closure): void
        {
            \Closure::bind($closure, new Other(), Other::class)();
        }
    }

    final class Host
    {
        public function hello(): void
        {
        }

        public function go(Binder $binder): void
        {
            $binder->rebind(function (): void { $this->hello(); });
        }

        public function goHeld(): void
        {
            $held = function (): void { $this->hello(); };
            (new Binder())->rebind($held);
        }
    }

    trait Greeter
    {
        public function greet(): string
        {
            return 'trait';
        }
    }

    // `as private`: a call in Hides runs its own copy, whatever Shows declares.
    class Hides
    {
        use Greeter {
            greet as private;
            greet as private hidden;
        }

        public function speak(): void
        {
            $this->greet();
            $this->hidden();
        }
    }

    final class Shows extends Hides
    {
        public function greet(): string
        {
            return 'shows';
        }

        public function hidden(): string
        {
            return 'shows';
        }
    }

    trait Secret
    {
        private function reveal(): string
        {
            return 'trait';
        }
    }

    // `as public`: the trait's private method is Opens', which Opened overrides.
    class Opens
    {
        use Secret {
            reveal as public;
            reveal as public uncover;
        }

        public function show(): void
        {
            $this->reveal();
            $this->uncover();
        }
    }

    final class Opened extends Opens
    {
        public function reveal(): string
        {
            return 'opened';
        }

        public function uncover(): string
        {
            return 'opened';
        }
    }

    function countdown(int $n): int
    {
        return $n > 0 ? countdown($n - 1) : 0;
    }

    function outer(): void
    {
        function inner(): int
        {
            return 1;
        }
        inner();
    }

    function entry(): void
    {
        (new Tag())->label();
        $bag = new Bag();
        $bag->size();
        $bag->sum();
        $steps = new Steps();
        $steps->run(true);
        new Kid();
        countdown(2);
        outer();
        (new Keyed())->run();
        (new Host())->go(new Binder());
        (new Host())->goHeld();
        (new Shows())->speak();
        (new Opens())->show();
        (new Opened())->show();
    }
}

namespace {

    function more_entry(): void
    {
        \App\More\entry();
        App\More\countdown(1);
    }
}
