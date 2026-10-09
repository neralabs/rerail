<?php
                    /**
                     * @param mixed|null $mixed_or_null
                     */
                    function foo($mixed_or_null): Exception {
                        /**
                         * @psalm-suppress MixedArgument
                         */
                        return new Exception($mixed_or_null);
                    }