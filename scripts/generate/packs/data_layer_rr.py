"""Import pinned .rr libraries and rule tables from the syntax worktree."""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / 'scripts/dsl/data_layer_rr'))
from migration import main

if __name__ == '__main__':
    main()
