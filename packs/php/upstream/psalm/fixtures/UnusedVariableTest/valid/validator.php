<?php
                    /**
                     * @param bool $b
                     */
                    function validate($b, string $source) : void {
                        /** @var bool|string $b */
                        if (!is_bool($b)) {
                            $source = $b;
                            $b = false;
                        }

                        /**
                         * test to ensure $b is only type bool and not bool|string anymore
                         * after we set $b = false; inside the condition above
                         * @psalm-suppress TypeDoesNotContainType
                         */
                        if (!is_bool($b)) {
                            echo "this should not happen";
                        }

                        print_r($source);
                    }