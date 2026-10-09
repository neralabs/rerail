<?php
                    /** @return mixed */
                    function f() {
                        $a = random_int(0, 10) >= 5 ? true : false;

                        $b = (int) $a;

                        return $b;
                    }
                