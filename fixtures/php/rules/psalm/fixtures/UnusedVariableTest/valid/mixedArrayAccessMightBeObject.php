<?php
                    function takesResults(array $arr) : void {
                        /**
                         * @psalm-suppress MixedAssignment
                         */
                        foreach ($arr as $item) {
                            /**
                             * @psalm-suppress MixedArrayAccess
                             * @psalm-suppress MixedArrayAssignment
                             */
                            $item[0] = $item[1];
                        }
                    }