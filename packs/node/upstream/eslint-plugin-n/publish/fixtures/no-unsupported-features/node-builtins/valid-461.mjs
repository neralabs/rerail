
                        const { DatabaseSync } = process.getBuiltinModule('node:sqlite');
                        const database = new DatabaseSync(':memory:');
                    