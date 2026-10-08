<?php
                    function keys(): array {
                        return ["foo", "bar"];
                    }

                    /** @var mixed $k */
                    foreach (keys() as $k) {
                        echo gettype($k);
                    }