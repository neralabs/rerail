// `node --import <this file's URL>`: registers tsx resolved from the repository, so a process started with its working
// directory in a scenario checkout (outside the repository, where a bare `--import tsx` would not resolve) still runs
// the TypeScript sources.
import "tsx";
