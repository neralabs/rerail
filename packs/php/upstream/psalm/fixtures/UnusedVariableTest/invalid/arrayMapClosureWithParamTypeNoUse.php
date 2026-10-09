<?php
                    $a = [1, 2, 3];

                    $b = array_map(
                        function(int $i) {
                            return rand(0, 5);
                        },
                        $a
                    );

                    foreach ($b as $c) {
                        echo $c;
                    }