<?php
                    class A {
                        public static function makeArray() : array {
                            return ["hello"];
                        }
                    }

                    $arr = A::makeArray();

                    foreach ($arr as $a) {
                        echo $a;
                    }