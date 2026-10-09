<?php
                    /**
                     * @param list<int> $keys
                     * @param int $key
                     */
                    function error2(array $keys, int $key): int
                    {
                        if ($key === 1) {}

                        do {
                            $nextKey = $keys[++$key] ?? null;
                        } while ($nextKey === null);

                        return $nextKey;
                    }