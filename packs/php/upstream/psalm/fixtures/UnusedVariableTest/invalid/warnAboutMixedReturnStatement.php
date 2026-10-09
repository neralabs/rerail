<?php
                    function makeArray() : array {
                        return ["hello"];
                    }

                    function foo() : string {
                        $arr = makeArray();

                        /** @psalm-suppress MixedAssignment */
                        foreach ($arr as $a) {
                            return $a;
                        }

                        return "";
                    }