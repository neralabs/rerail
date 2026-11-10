//! randomforest_kernels_grid.cc, in Rust.
//!
//! Jeremy Barnes, 8 September 2021. Copyright (c) 2021 Jeremy Barnes. This file is part of MLDB. Copyright 2016
//! mldb.ai inc. Apache-2.0 (see the LICENSE at `accel/src/forest/mldb/`).
//!
//! Kernels for the random forest algorithm, as grid kernels: each registration names the kernel's dimensions,
//! parameters, tuneables, constraints and grid expressions, and its compute function in the `randomforest_kernels`
//! library. `kernel(name)` specializes a registration against its Metal function once (upstream's
//! `registerGridComputeKernel` and the context's `getKernel`). `ComputeDevice::host()` on two registrations
//! (unused by the grid runtime) is not kept.

use crate::forest::compute_kernel_grid::{GridComputeKernelSpecialization, GridComputeKernelTemplate};
use crate::forest::randomforest_types::{WIndexed, W};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// Default of 5.5k allows 8 parallel workgroups for a 48k SM when accounting for 0.5k of local memory for the
/// kernels. On Nvidia, with 32 registers/work item and 256 work items/workgroup (8 warps of 32 threads), we use
/// 32 * 256 * 8 = 64k registers, which means full occupancy. (`RF_LOCAL_BUCKET_MEM` in the environment.)
pub fn rf_local_bucket_mem() -> i64 {
    std::env::var("RF_LOCAL_BUCKET_MEM").ok().and_then(|v| v.parse().ok()).unwrap_or(5500)
}

/// TODO: device query (upstream).
pub const MAX_WORK_GROUP_SIZE: i64 = 256;

const W_SIZE: i64 = std::mem::size_of::<W>() as i64;
const WINDEXED_SIZE: i64 = std::mem::size_of::<WIndexed>() as i64;

/// The registration of grid kernel `name`.
pub fn template(name: &str) -> Option<GridComputeKernelTemplate> {
    let mut r = GridComputeKernelTemplate::new(name);
    match name {
        "decodeRows" => {
            r.add_dimension_max("r", "nr", 256).allow_grid_padding();
            r.add_parameter("rowData", "r", "u64[rowDataLength]")
                .add_parameter("rowDataLength", "r", "u32")
                .add_parameter("weightBits", "r", "u16")
                .add_parameter("exampleNumBits", "r", "u16")
                .add_parameter("numRows", "r", "u32")
                .add_parameter("weightFormat", "r", "WeightFormat")
                .add_parameter("weightMultiplier", "r", "f32")
                .add_parameter("weightData", "r", "f32[weightDataLength]")
                .add_parameter("decodedRowsOut", "w", "f32[numRows]");
            r.add_tuneable("threadsPerBlock", MAX_WORK_GROUP_SIZE).add_tuneable("blocksPerGrid", 16);
            r.set_grid_expression("[blocksPerGrid]", "[threadsPerBlock]");
            r.set_compute_function("randomforest_kernels", "decompressRowsKernel");
        }
        "testFeature" => {
            r.add_dimension("fidx", "naf").add_dimension("rowNum", "numRows");
            r.add_parameter("decodedRows", "r", "f32[numRows]")
                .add_parameter("numRows", "r", "u32")
                .add_parameter("bucketData", "r", "u32[bucketDataLength]")
                .add_parameter("bucketDataOffsets", "r", "u32[nf + 1]")
                .add_parameter("bucketNumbers", "r", "u32[nf + 1]")
                .add_parameter("bucketEntryBits", "r", "u32[nf]")
                .add_parameter("activeFeatureList", "r", "u32[naf]")
                .add_parameter("partitionBuckets", "rw", "W32[numBuckets]");
            r.add_tuneable("maxLocalBuckets", rf_local_bucket_mem() / W_SIZE).add_tuneable("threadsPerBlock", MAX_WORK_GROUP_SIZE).add_tuneable("blocksPerGrid", 32);
            r.add_parameter("w", "w", "W[maxLocalBuckets]");
            r.set_grid_expression("[naf,blocksPerGrid]", "[1,threadsPerBlock]").allow_grid_padding();
            r.set_compute_function("randomforest_kernels", "testFeatureKernel");
        }
        "getPartitionSplits" => {
            r.add_dimension("fidx", "naf");
            r.add_parameter("treeTrainingInfo", "r", "TreeTrainingInfo[1]")
                .add_parameter("bucketNumbers", "r", "u32[nf + 1]")
                .add_parameter("activeFeatureList", "r", "u32[naf]")
                .add_parameter("featureIsOrdinal", "r", "u32[nf]")
                .add_parameter("buckets", "r", "W32[numActiveBuckets * nap]")
                .add_parameter("wAll", "r", "W32[nap]")
                .add_parameter("featurePartitionSplitsOut", "w", "PartitionSplit[nap * naf]")
                .add_parameter("treeDepthInfo", "r", "TreeDepthInfo[1]");
            r.add_tuneable("numPartitionsInParallel", MAX_WORK_GROUP_SIZE).add_tuneable("wLocalSize", rf_local_bucket_mem() / WINDEXED_SIZE);
            r.add_parameter("wLocal", "w", "WIndexed[wLocalSize]");
            r.set_grid_expression("[1,naf,numPartitionsInParallel]", "[64,1,1]");
            r.set_compute_function("randomforest_kernels", "getPartitionSplitsKernel");
        }
        "bestPartitionSplit" => {
            r.add_parameter("treeTrainingInfo", "r", "TreeTrainingInfo[1]")
                .add_parameter("treeDepthInfo", "r", "TreeDepthInfo[1]")
                .add_parameter("activeFeatureList", "r", "u32[numActiveFeatures]")
                .add_parameter("featurePartitionSplits", "r", "PartitionSplit[numActivePartitions * numActiveFeatures]")
                .add_parameter("partitionIndexes", "r", "PartitionIndex[npi]")
                .add_parameter("allPartitionSplitsOut", "w", "IndexedPartitionSplit[maxPartitions]");
            r.add_tuneable("numPartitionsAtOnce", MAX_WORK_GROUP_SIZE);
            r.set_grid_expression("[numPartitionsAtOnce]", "[1]");
            r.set_compute_function("randomforest_kernels", "bestPartitionSplitKernel");
        }
        "assignPartitionNumbers" => {
            r.add_parameter("treeTrainingInfo", "r", "TreeTrainingInfo[1]")
                .add_parameter("treeDepthInfo", "r", "TreeDepthInfo[1]")
                .add_parameter("allPartitionSplits", "r", "IndexedPartitionSplit[np]")
                .add_parameter("partitionIndexesOut", "w", "PartitionIndex[maxActivePartitions]")
                .add_parameter("partitionInfoOut", "w", "PartitionInfo[numActivePartitions]")
                .add_parameter("smallSideIndexesOut", "w", "u8[maxActivePartitions]")
                .add_parameter("smallSideIndexToPartitionOut", "w", "u16[256]");
            r.set_grid_expression("[1]", "[32]");
            r.set_compute_function("randomforest_kernels", "assignPartitionNumbersKernel");
        }
        "clearBuckets" => {
            r.add_dimension("bucket", "numActiveBuckets");
            r.add_parameter("treeTrainingInfo", "r", "TreeTrainingInfo[1]")
                .add_parameter("treeDepthInfo", "r", "TreeDepthInfo[1]")
                .add_parameter("bucketsOut", "w", "W32[numActiveBuckets * numActivePartitions]")
                .add_parameter("wAllOut", "w", "W32[numActivePartitions]")
                .add_parameter("numNonZeroDirectionIndices", "w", "u32[1]")
                .add_parameter("smallSideIndexes", "r", "u8[numActivePartitions]");
            r.allow_grid_padding();
            r.add_tuneable("gridBlockSize", 64).add_tuneable("numPartitionsAtOnce", MAX_WORK_GROUP_SIZE);
            r.set_grid_expression("[numPartitionsAtOnce,ceilDiv(numActiveBuckets,gridBlockSize)]", "[1,gridBlockSize]");
            r.set_compute_function("randomforest_kernels", "clearBucketsKernel");
        }
        "predictForest" => {
            r.add_dimension("r", "numRows");
            r.add_parameter("numRows", "r", "u32")
                .add_parameter("numTrees", "r", "u32")
                .add_parameter("maxDepth", "r", "u32")
                .add_parameter("bucketData", "r", "u32[bucketDataLength]")
                .add_parameter("bucketDataOffsets", "r", "u32[nf + 1]")
                .add_parameter("bucketEntryBits", "r", "u32[nf]")
                .add_parameter("nodes", "r", "PredictNode[numNodes]")
                .add_parameter("treeRoots", "r", "i32[numTrees]")
                .add_parameter("treeWeights", "r", "f32[numTrees]")
                .add_parameter("leafPreds", "r", "f32[numLeafPreds]")
                .add_parameter("predictionsOut", "w", "f32[numRows * 2]");
            r.add_tuneable("threadsPerBlock", MAX_WORK_GROUP_SIZE);
            r.set_grid_expression("[ceilDiv(numRows,threadsPerBlock)]", "[threadsPerBlock]").allow_grid_padding();
            r.set_compute_function("randomforest_kernels", "predictForestKernel");
        }
        "updatePartitionNumbers" => {
            r.add_dimension("r", "numRows");
            r.add_parameter("treeTrainingInfo", "r", "TreeTrainingInfo[1]")
                .add_parameter("treeDepthInfo", "r", "TreeDepthInfo[1]")
                .add_parameter("partitions", "r", "RowPartitionInfo[numRows]")
                .add_parameter("directions", "w", "u32[(numRows+31)/32]")
                .add_parameter("numNonZeroDirectionIndices", "rw", "u32[1]")
                .add_parameter("nonZeroDirectionIndices", "w", "UpdateWorkEntry[numRows / 2 + 2]")
                .add_parameter("smallSideIndexes", "r", "u8[numActivePartitions]")
                .add_parameter("allPartitionSplits", "r", "IndexedPartitionSplit[naps]")
                .add_parameter("partitionInfo", "r", "PartitionInfo[np]")
                .add_parameter("bucketData", "r", "u32[bucketDataLength]")
                .add_parameter("bucketDataOffsets", "r", "u32[nf + 1]")
                .add_parameter("bucketNumbers", "r", "u32[nf + 1]")
                .add_parameter("bucketEntryBits", "r", "u32[nf]")
                .add_parameter("featureIsOrdinal", "r", "u32[nf]")
                .add_parameter("decodedRows", "r", "f32[numRows]");
            r.add_tuneable("threadsPerBlock", MAX_WORK_GROUP_SIZE).add_tuneable("blocksPerGrid", 96);
            r.allow_grid_padding();
            r.set_grid_expression("[blocksPerGrid]", "[threadsPerBlock]");
            r.set_compute_function("randomforest_kernels", "updatePartitionNumbersKernel");
        }
        "updateBuckets" => {
            r.add_dimension("r", "numRows").add_dimension("fidx_plus_1", "naf_plus_1");
            r.add_parameter("treeTrainingInfo", "r", "TreeTrainingInfo[1]")
                .add_parameter("treeDepthInfo", "r", "TreeDepthInfo[1]")
                .add_parameter("partitions", "r", "RowPartitionInfo[numRows]")
                .add_parameter("directions", "r", "u32[(numRows + 31)/32]")
                .add_parameter("numNonZeroDirectionIndices", "r", "u32[1]")
                .add_parameter("nonZeroDirectionIndices", "r", "UpdateWorkEntry[numRows / 2 + 2]")
                .add_parameter("buckets", "w", "W32[numActiveBuckets * numActivePartitions]")
                .add_parameter("wAll", "w", "W32[numActivePartitions]")
                .add_parameter("smallSideIndexes", "r", "u8[numActivePartitions]")
                .add_parameter("smallSideIndexToPartition", "r", "u16[256]")
                .add_parameter("decodedRows", "r", "f32[nr]")
                .add_parameter("bucketData", "r", "u32[bucketDataLength]")
                .add_parameter("bucketDataOffsets", "r", "u32[nf + 1]")
                .add_parameter("bucketNumbers", "r", "u32[nf + 1]")
                .add_parameter("bucketEntryBits", "r", "u32[nf]")
                .add_parameter("activeFeatureList", "r", "u32[numActiveFeatures]")
                .add_parameter("featureIsOrdinal", "r", "u32[nf]");
            r.add_tuneable("maxLocalBuckets", rf_local_bucket_mem() / W_SIZE).add_tuneable("threadsPerBlock", MAX_WORK_GROUP_SIZE).add_tuneable("blocksPerGrid", 32);
            r.add_parameter("wLocal", "w", "W[maxLocalBuckets]");
            r.add_constraint("naf_plus_1", "==", "numActiveFeatures + 1", "help the solver");
            r.add_constraint("numActiveFeatures", "==", "naf_plus_1 - 1", "help the solver");
            r.set_grid_expression("[blocksPerGrid,numActiveFeatures+1]", "[threadsPerBlock,1]");
            r.allow_grid_padding();
            r.set_compute_function("randomforest_kernels", "updateBucketsKernel");
        }
        "fixupBuckets" => {
            r.add_dimension("bucket", "numActiveBuckets");
            r.add_parameter("treeTrainingInfo", "r", "TreeTrainingInfo[=1]")
                .add_parameter("treeDepthInfo", "r", "TreeDepthInfo[=1]")
                .add_parameter("buckets", "rw", "W32[numActiveBuckets * newNumPartitions]")
                .add_parameter("wAll", "rw", "W32[newNumPartitions]")
                .add_parameter("partitionInfo", "r", "PartitionInfo[np]")
                .add_parameter("smallSideIndexes", "r", "u8[newNumPartitions]");
            r.add_tuneable("gridBlockSize", 64).add_tuneable("numPartitionsAtOnce", MAX_WORK_GROUP_SIZE);
            r.allow_grid_padding();
            r.set_grid_expression("[numPartitionsAtOnce,ceilDiv(numActiveBuckets,gridBlockSize)]", "[1,gridBlockSize]");
            r.set_compute_function("randomforest_kernels", "fixupBucketsKernel");
        }
        _ => return None,
    }
    Some(r)
}

/// The registered grid kernels, in the order a tree's training runs them.
pub const GRID_KERNELS: [&str; 10] = ["decodeRows", "testFeature", "getPartitionSplits", "bestPartitionSplit", "assignPartitionNumbers", "clearBuckets", "updatePartitionNumbers", "updateBuckets", "fixupBuckets", "predictForest"];

/// Grid kernel `name`, specialized against its Metal function (once).
pub fn kernel(name: &str) -> Result<&'static GridComputeKernelSpecialization, String> {
    static KS: OnceLock<Mutex<HashMap<String, &'static GridComputeKernelSpecialization>>> = OnceLock::new();
    let mut ks = KS.get_or_init(Default::default).lock().unwrap();
    if let Some(k) = ks.get(name) {
        return Ok(k);
    }
    let t = template(name).ok_or_else(|| format!("no grid kernel {name}"))?;
    let f = crate::forest::randomforest_kernels_metal::function(&t.function_name)?;
    let k: &'static GridComputeKernelSpecialization = Box::leak(Box::new(GridComputeKernelSpecialization::new(&t, f)?));
    ks.insert(name.to_string(), k);
    Ok(k)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every registration specializes against its Metal function: each reflected argument finds its parameter,
    /// its `args` structure's fields, or its threadgroup length.
    #[test]
    fn grid_kernels_specialize_on_metal() {
        if crate::gpu::shared::get().is_none() {
            eprintln!("no Metal device: skipped");
            return;
        }
        for name in GRID_KERNELS {
            let k = kernel(name).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(!k.bind_actions.is_empty(), "{name}");
        }
    }

    use crate::forest::bucket::WritableBucketList;
    use crate::forest::compute_kernel_grid::ComputeKernelArgument::{self, Buffer as B, Float, Int};
    use crate::forest::compute_kernel_metal::MetalComputeQueue;
    use crate::forest::memory_region::{MemorySerializer, Pod};
    use crate::forest::randomforest_kernels as host;
    use crate::forest::randomforest_types::*;

    fn buf<T: Pod>(v: &[T]) -> metal::Buffer {
        let shared = crate::gpu::shared::get().unwrap();
        let n = std::mem::size_of_val(v).max(16);
        let b = shared.device.new_buffer(n as u64, metal::MTLResourceOptions::StorageModeShared);
        unsafe { std::ptr::copy_nonoverlapping(v.as_ptr().cast::<u8>(), b.contents().cast::<u8>(), std::mem::size_of_val(v)) };
        b
    }

    fn read<T: Pod + Clone>(b: &metal::Buffer, n: usize) -> Vec<T> {
        unsafe { std::slice::from_raw_parts(b.contents().cast::<T>(), n).to_vec() }
    }

    fn run(name: &str, args: &[(&str, ComputeKernelArgument)], grid: &[u32]) {
        let shared = crate::gpu::shared::get().unwrap();
        let k = kernel(name).unwrap_or_else(|e| panic!("{e}"));
        let q = MetalComputeQueue::new(&shared.queue);
        {
            let mut ctx = q.new_bind_context(k.function);
            k.enqueue(args, grid, &mut ctx).unwrap_or_else(|e| panic!("{e}"));
        }
        q.finish().unwrap_or_else(|e| panic!("{name}: {e}"));
    }

    /// One level of training on Metal, through the nine grid kernels, checked after each against the kernels' host
    /// forms over the same inputs: the decoded rows, the root's buckets, the splits per feature and the best one,
    /// the partition numbering, the rows' new partitions (the tree evaluated on the rows) with the work list of
    /// the rows that moved, and the children's buckets after the transfer and the fixup.
    #[test]
    fn one_level_on_metal_matches_the_host_kernels() {
        if crate::gpu::shared::get().is_none() {
            eprintln!("no Metal device: skipped");
            return;
        }
        let mut s = MemorySerializer;
        let nr = 1000u32;
        let label = |r: u32| r % 8 >= 3 || r % 13 == 0;
        let feature_nb = [8u32, 3, 50];
        let bucket_of = |f: usize, r: u32| match f {
            0 => r % 8,
            1 => r % 3,
            _ => (r * 7) % 50,
        };
        // Weights as an int multiple: the device's decodeWeight takes every weight as one (`|| true` upstream), and
        // its label mask is 32 bits wide, so a 32-bit float weight would lose the label.
        let multiplier = 1.0 / 1024.0;
        let enc = WeightEncoder { weight_format: WeightFormat::WfIntMultiple, weight_bits: 5, weight_multiplier: multiplier, ..Default::default() };
        let mut w = RowWriter::new(&enc, nr as usize, nr as usize, &mut s, true, 1.0);
        for r in 0..nr {
            w.add_row_values(label(r), 1.0 / 1024.0, r).unwrap();
        }
        let rows = w.freeze(&mut s);
        let nf = feature_nb.len();

        // decodeRows.
        let decoded = host::decode_rows(&rows);
        let d_decoded = buf(&vec![5f32; nr as usize]);
        let row_data = buf(rows.row_data.data());
        let weight_data = buf(&[0f32; 4]);
        run("decodeRows", &[("rowData", B(&row_data)), ("rowDataLength", Int(rows.row_data.length() as i64)), ("weightBits", Int(5)), ("exampleNumBits", Int(0)), ("numRows", Int(nr as i64)), ("weightFormat", Int(WeightFormat::WfIntMultiple as i64)), ("weightMultiplier", Float(multiplier as f64)), ("weightData", B(&weight_data)), ("decodedRowsOut", B(&d_decoded))], &[nr]);
        assert_eq!(read::<f32>(&d_decoded, nr as usize), decoded, "decodeRows");

        let mut all_bucket_data: Vec<u32> = Vec::new();
        let (mut offsets, mut numbers, mut entry_bits) = (vec![0u32], vec![0u32], vec![]);
        for (f, &nb) in feature_nb.iter().enumerate() {
            let mut bl = WritableBucketList::new(nr as usize, nb, &mut s);
            for r in 0..nr {
                bl.write(bucket_of(f, r));
            }
            let frozen = bl.freeze(&mut s);
            entry_bits.push(frozen.entry_bits as u32);
            all_bucket_data.extend_from_slice(frozen.storage.data());
            offsets.push(all_bucket_data.len() as u32);
            numbers.push(numbers.last().unwrap() + nb);
        }
        let total = *numbers.last().unwrap();
        let active: Vec<u32> = (0..nf as u32).collect();
        let ordinal = vec![1u32, 0, 1];
        let max_parts = 8usize;
        let tti = [TreeTrainingInfo { num_rows: nr, max_depth: 4, num_active_features: nf as u16, num_features: nf as u16, max_num_active_partitions: max_parts as u16, num_active_buckets: total, ..Default::default() }];
        let mut tdi = [TreeDepthInfo::default()];
        let (d_bucket_data, d_offsets, d_numbers, d_entry_bits, d_active, d_ordinal) = (buf(&all_bucket_data), buf(&offsets), buf(&numbers), buf(&entry_bits), buf(&active), buf(&ordinal));
        let d_tti = buf(&tti);

        // testFeature: the root's buckets.
        let mut buckets = vec![W::default(); max_parts * total as usize];
        for fidx in 0..nf as u32 {
            host::test_feature_kernel_host(fidx, nf as u32, 0..nr, &decoded, nr, &all_bucket_data, &offsets, &numbers, &entry_bits, &active, &mut buckets);
        }
        let d_buckets = buf(&vec![W::default(); max_parts * total as usize]);
        run("testFeature", &[("decodedRows", B(&d_decoded)), ("numRows", Int(nr as i64)), ("bucketData", B(&d_bucket_data)), ("bucketDataOffsets", B(&d_offsets)), ("bucketNumbers", B(&d_numbers)), ("bucketEntryBits", B(&d_entry_bits)), ("activeFeatureList", B(&d_active)), ("partitionBuckets", B(&d_buckets))], &[nf as u32, nr]);
        assert_eq!(read::<W>(&d_buckets, total as usize), buckets[..total as usize], "testFeature");

        let mut w_all = vec![W::default(); max_parts];
        for &d in &decoded {
            w_all[0].add_weight(d < 0.0, d.abs());
        }
        let d_w_all = buf(&w_all);
        let d_tdi = buf(&tdi);

        // getPartitionSplits.
        let mut splits = vec![PartitionSplit::default(); max_parts * nf];
        for fidx in 0..nf as u32 {
            host::get_partition_splits_kernel(fidx, nf as u32, &tti, &numbers, &active, &ordinal, &buckets, &w_all, &mut splits, &tdi);
        }
        let d_splits = buf(&vec![PartitionSplit::default(); max_parts * nf]);
        run("getPartitionSplits", &[("treeTrainingInfo", B(&d_tti)), ("bucketNumbers", B(&d_numbers)), ("activeFeatureList", B(&d_active)), ("featureIsOrdinal", B(&d_ordinal)), ("buckets", B(&d_buckets)), ("wAll", B(&d_w_all)), ("featurePartitionSplitsOut", B(&d_splits)), ("treeDepthInfo", B(&d_tdi))], &[nf as u32]);
        assert_eq!(read::<PartitionSplit>(&d_splits, nf), splits[..nf], "getPartitionSplits");

        // bestPartitionSplit.
        let mut all_splits = vec![IndexedPartitionSplit::default(); 64];
        host::best_partition_split_kernel(&tti, &tdi, &active, &splits, &[PartitionIndex::root()], &mut all_splits);
        let d_indexes = buf(&vec![PartitionIndex::none(); max_parts]);
        let d_all_splits = buf(&vec![IndexedPartitionSplit::default(); 64]);
        run("bestPartitionSplit", &[("treeTrainingInfo", B(&d_tti)), ("treeDepthInfo", B(&d_tdi)), ("activeFeatureList", B(&d_active)), ("featurePartitionSplits", B(&d_splits)), ("partitionIndexes", B(&d_indexes)), ("allPartitionSplitsOut", B(&d_all_splits))], &[]);
        assert_eq!(read::<IndexedPartitionSplit>(&d_all_splits, 1), all_splits[..1], "bestPartitionSplit");
        assert_eq!(all_splits[0].split.feature, 0);

        // assignPartitionNumbers.
        let mut indexes = vec![PartitionIndex::none(); max_parts];
        let mut info = vec![PartitionInfo::default(); max_parts];
        let mut ssi = vec![0u8; max_parts];
        let mut ssi2p = vec![0u16; 256];
        host::assign_partition_numbers_kernel(&tti, &mut tdi, &all_splits, &mut indexes, &mut info, &mut ssi, &mut ssi2p);
        let (d_info, d_ssi, d_ssi2p) = (buf(&vec![PartitionInfo::default(); max_parts]), buf(&vec![0u8; max_parts]), buf(&vec![0u16; 256]));
        run("assignPartitionNumbers", &[("treeTrainingInfo", B(&d_tti)), ("treeDepthInfo", B(&d_tdi)), ("allPartitionSplits", B(&d_all_splits)), ("partitionIndexesOut", B(&d_indexes)), ("partitionInfoOut", B(&d_info)), ("smallSideIndexesOut", B(&d_ssi)), ("smallSideIndexToPartitionOut", B(&d_ssi2p))], &[]);
        assert_eq!(read::<TreeDepthInfo>(&d_tdi, 1), tdi, "assignPartitionNumbers: treeDepthInfo");
        let nap = tdi[0].num_active_partitions as usize;
        assert_eq!(read::<PartitionIndex>(&d_indexes, nap), indexes[..nap], "partitionIndexes");
        assert_eq!(read::<PartitionInfo>(&d_info, 1), info[..1], "partitionInfo");
        assert_eq!(read::<u8>(&d_ssi, nap), ssi[..nap], "smallSideIndexes");

        // clearBuckets.
        let mut nndi = [0u32];
        host::clear_buckets_kernel(0..total, &tti, &tdi, &mut buckets, &mut w_all, &mut nndi, &ssi);
        let d_nndi = buf(&[7u32]);
        run("clearBuckets", &[("treeTrainingInfo", B(&d_tti)), ("treeDepthInfo", B(&d_tdi)), ("bucketsOut", B(&d_buckets)), ("wAllOut", B(&d_w_all)), ("numNonZeroDirectionIndices", B(&d_nndi)), ("smallSideIndexes", B(&d_ssi))], &[total]);
        assert_eq!(read::<W>(&d_buckets, nap * total as usize), buckets[..nap * total as usize], "clearBuckets: buckets");
        assert_eq!(read::<W>(&d_w_all, nap), w_all[..nap], "clearBuckets: wAll");
        assert_eq!(read::<u32>(&d_nndi, 1), nndi, "clearBuckets: numNonZeroDirectionIndices");

        // updatePartitionNumbers: the split applied to every row.
        let mut partitions = vec![RowPartitionInfo::default(); nr as usize];
        let mut work = vec![UpdateWorkEntry::default(); nr as usize / 2 + 2];
        host::update_partition_numbers_kernel(0..nr, &tti, &tdi, &mut partitions, &mut [], &mut nndi, &mut work, &ssi, &all_splits, &info, &all_bucket_data, &offsets, &numbers, &entry_bits, &ordinal, &decoded);
        let (d_partitions, d_directions, d_work) = (buf(&vec![RowPartitionInfo::default(); nr as usize]), buf(&vec![0u32; nr.div_ceil(32) as usize]), buf(&vec![UpdateWorkEntry::default(); nr as usize / 2 + 2]));
        run("updatePartitionNumbers", &[("treeTrainingInfo", B(&d_tti)), ("treeDepthInfo", B(&d_tdi)), ("partitions", B(&d_partitions)), ("directions", B(&d_directions)), ("numNonZeroDirectionIndices", B(&d_nndi)), ("nonZeroDirectionIndices", B(&d_work)), ("smallSideIndexes", B(&d_ssi)), ("allPartitionSplits", B(&d_all_splits)), ("partitionInfo", B(&d_info)), ("bucketData", B(&d_bucket_data)), ("bucketDataOffsets", B(&d_offsets)), ("bucketNumbers", B(&d_numbers)), ("bucketEntryBits", B(&d_entry_bits)), ("featureIsOrdinal", B(&d_ordinal)), ("decodedRows", B(&d_decoded))], &[nr]);
        assert_eq!(read::<RowPartitionInfo>(&d_partitions, nr as usize), partitions, "updatePartitionNumbers: partitions");
        assert_eq!(read::<u32>(&d_nndi, 1), nndi, "updatePartitionNumbers: count");
        let n = nndi[0] as usize;
        let key = |e: &UpdateWorkEntry| (e.row, e.partition, e.small_side_index, e.decoded_row.to_bits());
        let mut dw: Vec<_> = read::<UpdateWorkEntry>(&d_work, n).iter().map(key).collect();
        let mut hw: Vec<_> = work[..n].iter().map(key).collect();
        dw.sort();
        hw.sort();
        assert_eq!(dw, hw, "updatePartitionNumbers: work (unordered)");

        // updateBuckets and fixupBuckets.
        for fidxp1 in 0..=nf as u32 {
            host::update_buckets_kernel(0..nr, fidxp1, nf as u32 + 1, &tti, &tdi, &partitions, &[], &nndi, &work, &mut buckets, &mut w_all, &ssi, &ssi2p, &decoded, &all_bucket_data, &offsets, &numbers, &entry_bits, &active, &ordinal);
        }
        run("updateBuckets", &[("treeTrainingInfo", B(&d_tti)), ("treeDepthInfo", B(&d_tdi)), ("partitions", B(&d_partitions)), ("directions", B(&d_directions)), ("numNonZeroDirectionIndices", B(&d_nndi)), ("nonZeroDirectionIndices", B(&d_work)), ("buckets", B(&d_buckets)), ("wAll", B(&d_w_all)), ("smallSideIndexes", B(&d_ssi)), ("smallSideIndexToPartition", B(&d_ssi2p)), ("decodedRows", B(&d_decoded)), ("bucketData", B(&d_bucket_data)), ("bucketDataOffsets", B(&d_offsets)), ("bucketNumbers", B(&d_numbers)), ("bucketEntryBits", B(&d_entry_bits)), ("activeFeatureList", B(&d_active)), ("featureIsOrdinal", B(&d_ordinal))], &[nr, nf as u32 + 1]);
        assert_eq!(read::<W>(&d_buckets, nap * total as usize), buckets[..nap * total as usize], "updateBuckets: buckets");
        assert_eq!(read::<W>(&d_w_all, nap), w_all[..nap], "updateBuckets: wAll");
        host::fixup_buckets_kernel(0..total, &tti, &tdi, &mut buckets, &mut w_all, &info, &ssi);
        run("fixupBuckets", &[("treeTrainingInfo", B(&d_tti)), ("treeDepthInfo", B(&d_tdi)), ("buckets", B(&d_buckets)), ("wAll", B(&d_w_all)), ("partitionInfo", B(&d_info)), ("smallSideIndexes", B(&d_ssi))], &[total]);
        assert_eq!(read::<W>(&d_buckets, nap * total as usize), buckets[..nap * total as usize], "fixupBuckets: buckets");
        assert_eq!(read::<W>(&d_w_all, nap), w_all[..nap], "fixupBuckets: wAll");
    }
}
