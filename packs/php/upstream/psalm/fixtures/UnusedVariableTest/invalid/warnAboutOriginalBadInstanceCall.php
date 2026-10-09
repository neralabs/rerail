<?php
                    class A {
                        public function makeArray() : array {
                            return ["hello"];
                        }
                    }

                    $arr = (new A)->makeArray();

                    foreach ($arr as $a) {
                        echo $a;
                    }