<?php
                    function reindex(array $arr, string $methodName): array {
                        $ret = [];

                        foreach ($arr as $element) {
                            $ret[$element->$methodName()] = true;
                        }

                        return $ret;
                    }