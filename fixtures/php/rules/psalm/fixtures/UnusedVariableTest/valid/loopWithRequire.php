<?php
                    /**
                     * @psalm-suppress UnresolvableInclude
                     */
                    function foo(string $delta_file) : void {
                        while (rand(0, 1)) {
                            /**
                             * @var array<string, mixed>
                             */
                            $diff_call_map = require($delta_file);

                            foreach ($diff_call_map as $key => $_) {
                                $cased_key = strtolower($key);
                                echo $cased_key;
                            }
                        }
                    }