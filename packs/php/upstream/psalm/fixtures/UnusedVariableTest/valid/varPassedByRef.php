<?php
                    function foo(array $returned) : array {
                        $ancillary = &$returned;
                        $ancillary["foo"] = 5;
                        return $returned;
                    }