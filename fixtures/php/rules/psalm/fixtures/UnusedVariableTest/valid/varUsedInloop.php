<?php
                    class A {
                        public static function getA() : ?A {
                            return rand(0, 1) ? new A : null;
                        }
                    }

                    function foo(?A $a) : void {
                        while ($a) {
                            echo get_class($a);
                            $a = A::getA();
                        }
                    }