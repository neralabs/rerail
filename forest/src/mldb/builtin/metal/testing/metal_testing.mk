# This file is part of MLDB. Copyright 2015 mldb.ai inc. All rights reserved.
# Used here under the Apache License, Version 2.0: MLDB's license is https://github.com/mldbai/mldb/blob/e3caed6fa44782d08eb640702fd60e9a2576d939/LICENSE

$(eval $(call test,metal_reference_counting_test,vfs block types value_description vfs utils base arch command_expression mldb_metal_plugin any,boost))
