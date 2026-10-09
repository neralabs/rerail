<?php
                    /** @return array[] */
                    function makeArray() : array {
                        return [["hello"]];
                    }

                    $arr = makeArray();

                    foreach ($arr as $some_arr) {
                        foreach ($some_arr as $a) {
                            echo $a;
                        }
                    }