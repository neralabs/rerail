<?php
                    class A {
                        public function foo() : bool {
                            return true;
                        }
                    }

                    function getA() : ?A {
                        return rand(0, 1) ? new A() : null;
                    }

                    if (rand(0, 1)) {
                        if (!($a = getA()) || $a->foo()) {}
                        return;
                    }

                    if (!($a = getA()) || $a->foo()) {}