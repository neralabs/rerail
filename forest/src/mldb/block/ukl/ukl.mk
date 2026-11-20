# This file is part of MLDB. Copyright 2022 Jeremy Barnes. All rights reserved.
# Used here under the Apache License, Version 2.0: MLDB's license is https://github.com/mldbai/mldb/blob/e3caed6fa44782d08eb640702fd60e9a2576d939/LICENSE

$(warning CWD=$(CWD))

$(CWD)/ukl_auto_macros.h:	$(CWD)/ukl_auto_macro_gen.py
	$< > $@~ && mv $@~ $@

