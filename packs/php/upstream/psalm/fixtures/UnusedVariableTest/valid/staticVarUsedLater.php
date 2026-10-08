<?php
                    function use_static() : int {
                        static $x = null;
                        if ($x) {
                            return (int) $x;
                        }
                        $x = rand(0, 1);
                        return -1;
                    }