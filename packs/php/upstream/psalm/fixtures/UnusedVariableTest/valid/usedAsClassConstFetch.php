<?php
                    class A {
                        const bool something = false;

                        public function foo() : void {
                            $var = "something";

                            if (rand(0, 1)) {
                                static::{$var};
                            }
                        }
                    }