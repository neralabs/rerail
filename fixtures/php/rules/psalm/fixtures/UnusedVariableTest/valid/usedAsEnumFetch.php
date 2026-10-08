<?php
                    enum E {
                        case C;
                    }

                    class A {
                        public function foo() : void {
                            $var = "C";

                            if (rand(0, 1)) {
                                E::{$var};
                            }
                        }
                    }