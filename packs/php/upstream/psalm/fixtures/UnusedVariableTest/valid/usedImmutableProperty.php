<?php
                    /**
                     * @psalm-immutable
                     */
                    class Clause {
                        /**
                         * @var array<int, int>
                         */
                        public $b = [];
                    }

                    function foo(Clause $c, int $var): void {
                        $new_b = $c->b;

                        if (isset($c->b[0])) {
                            $new_b[$var] = 0;
                        }

                        if ($new_b) {}
                    }