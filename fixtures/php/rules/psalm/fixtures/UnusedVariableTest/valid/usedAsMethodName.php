<?php
                    class A {
                        public static function foo() : void {}
                    }

                    function foo() : void {
                        $method = "foo";
                        A::$method();
                    }