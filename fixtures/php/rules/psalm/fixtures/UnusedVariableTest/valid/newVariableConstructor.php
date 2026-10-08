<?php
                    /**
                     * @param class-string<ArrayObject> $type
                     */
                    function bar(string $type) : ArrayObject {
                        $data = [["foo"], ["bar"]];

                        /** @psalm-suppress UnsafeInstantiation */
                        return new $type($data[0]);
                    }