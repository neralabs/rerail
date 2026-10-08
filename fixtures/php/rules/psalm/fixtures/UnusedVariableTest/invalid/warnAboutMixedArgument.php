<?php
                    function makeArray() : array {
                        return ["hello"];
                    }

                    $arr = makeArray();

                    /** @psalm-suppress MixedAssignment */
                    foreach ($arr as $a) {
                        echo $a;
                    }