<?php
                    function makeArray() : array {
                        return ["hello"];
                    }

                    $arr = makeArray();

                    foreach ($arr as $a) {
                        echo $a;
                    }