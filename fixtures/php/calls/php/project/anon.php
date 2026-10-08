<?php
// An anonymous class: its constructor and methods.

namespace App\Anon;

function entry(): void
{
    $o = new class (3) {
        public function __construct(public int $n)
        {
            $this->setup();
        }

        private function setup(): void
        {
        }

        public function twice(): int
        {
            return $this->n * 2;
        }
    };
    $o->twice();
}
