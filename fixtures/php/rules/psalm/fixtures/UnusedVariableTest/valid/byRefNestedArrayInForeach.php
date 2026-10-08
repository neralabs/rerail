<?php
                    function foo(array $arr): array {
                        /**
                         * @psalm-suppress MixedAssignment
                         * @psalm-suppress MixedArrayAssignment
                         */
                        foreach ($arr as &$element) {
                            $b = 5;
                            $element[0] = $b;
                        }

                        return $arr;
                    }