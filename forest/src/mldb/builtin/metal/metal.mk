# This file is part of MLDB. Copyright 2015 mldb.ai inc. All rights reserved.
# Used here under the Apache License, Version 2.0: MLDB's license is https://github.com/mldbai/mldb/blob/e3caed6fa44782d08eb640702fd60e9a2576d939/LICENSE

# Metal loader

ifneq ($(PREMAKE),1)

# Metal library for random forests
$(eval $(call metal_library,base_kernels_metal,base_kernels.metal))

METAL_PLUGIN_SOURCE := \
	metal.cc compute_kernel_metal.cc

METAL_PLUGIN_LINK := \
	value_description \
	v8 \
	v8_libplatform \
	arch \
	utils \
	types \
	mldb_engine \
	log \
	any \
	rest \
	rest_entity \
	json_diff \
	logging \
	vfs \
	base \
	mldb_core \
	mldb_builtin_base \
	sql_types \
	http \
	sql_expression \
	Metal \
	block \
	command_expression \
	mtlpp \
	base_kernels_metal \
	runner \

$(eval $(call set_compile_option,$(METAL_PLUGIN_SOURCE),-Imldb/ext))
$(eval $(call library,mldb_metal_plugin,$(METAL_PLUGIN_SOURCE),value_description sql_expression $(METAL_PLUGIN_LINK)))

$(eval $(call include_sub_make,metal_testing,testing))

endif
