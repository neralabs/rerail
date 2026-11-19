/** randomforest_kernels_ukl.h                                 -*- C++ -*-
    Jeremy Barnes, 13 October 2018
    Copyright (c) 2018 mldb.ai inc.  All rights reserved.
    This file is part of MLDB. Copyright 2016 mldb.ai inc. All rights reserved.

    The random forest's kernels, written once in UKL for every backend (Rerail, decision 0440 step 2): each
    kernel's parameters are rows of access, type, name and bounds, and its body calls the shared implementation
    in randomforest_kernels_common.h. The Metal shim makes the kernels the Metal host binds by name (each
    parameter's name, as the hand-written Metal wrappers this file replaces named them); the CUDA shim makes them for NVRTC.

    Rows beyond MLDB's (decision 0440 §3.1):
    - ARGS: a structure of arguments the host fills field by field, constant (Metal: `constant const T &`; CUDA:
      by value); its fields are reflected for CUDA's host by UKL_REFLECT_STRUCT after its definition;
    - CONSTBUFFER: a buffer in Metal's constant address space (CUDA: `const T * __restrict__`);
    - SHARED: `T Name[Count]` in threadgroup memory, declared in the kernel's scope; the body gets a pointer;
    - TGID0..2: the threadgroup's position in the grid; LANE, NLANES: the SIMD lane and the group's width.
*/

#include <ukl>

#include "mldb/plugins/jml/randomforest_kernels_common.h"

// Take a bit-compressed representation of rows, and turn it into a decompressed version with two floats per
// row: its weights to false and true (a label's side has the weight; a regression row's target t, by example,
// gives w (1 - t) and w t). Upstream had one float, the sign giving the label and the magnitude the weight.
// A 1D kernel over the rows.

typedef struct DecompressRowsKernelArgs {
    uint32_t rowDataLength;
    uint16_t weightBits;
    uint16_t exampleNumBits;
    uint32_t numRows;
    WeightFormat weightFormat;
    float weightMultiplier;
    uint32_t numTargets;
} DecompressRowsKernelArgs;
UKL_REFLECT_STRUCT(DecompressRowsKernelArgs, rowDataLength, weightBits, exampleNumBits, numRows, weightFormat, weightMultiplier, numTargets)


DEFINE_KERNEL2(randomforest_kernels, decompressRowsKernel,
               ARGS,     DecompressRowsKernelArgs, args,           ,
               ROBUFFER, uint64_t,                 rowData,        "[rowDataLength]",
               ROBUFFER, float,                    weightData,     "[weightDataLength]",
               ROBUFFER, float,                    targets,        "[numTargets]",
               RWBUFFER, float,                    decodedRowsOut, "[numRows * 2]",
               GID0,     uint32_t,                 id,             ,
               GSZ0,     uint32_t,                 n,              )
{
    decompressRowsImpl(rowData, args.rowDataLength, args.weightBits, args.exampleNumBits, args.numRows,
                       args.weightFormat, args.weightMultiplier, weightData, args.numTargets, targets,
                       decodedRowsOut, id, n);
}

typedef struct TestFeatureArgs {
    uint32_t numRows;
    uint32_t maxLocalBuckets;
} TestFeatureArgs;
UKL_REFLECT_STRUCT(TestFeatureArgs, numRows, maxLocalBuckets)


DEFINE_KERNEL2(randomforest_kernels, testFeatureKernel,
               ARGS,     TestFeatureArgs, args,              ,
               ROBUFFER, float,           decodedRows,       "[numRows * 2]",
               ROBUFFER, uint32_t,        bucketData,        "[bucketDataLength]",
               ROBUFFER, uint32_t,        bucketDataOffsets, "[nf + 1]",
               ROBUFFER, uint32_t,        bucketNumbers,     "[nf + 1]",
               ROBUFFER, uint32_t,        bucketEntryBits,   "[nf]",
               ROBUFFER, uint32_t,        activeFeatureList, "[naf]",
               RWLOCAL,  W,               w,                 "[maxLocalBuckets]",
               RWBUFFER, W,               partitionBuckets,  "[numBuckets]",
               GID0,     uint32_t,        gid0,              ,
               GID1,     uint32_t,        gid1,              ,
               GSZ1,     uint32_t,        gsz1,              ,
               LID1,     uint32_t,        lid1,              ,
               LSZ1,     uint32_t,        lsz1,              )
{
    testFeatureImpl(decodedRows, args.numRows, bucketData, bucketDataOffsets, bucketNumbers,
                    bucketEntryBits, activeFeatureList, w, args.maxLocalBuckets, partitionBuckets,
                    gid0, lid1, lsz1, gid1, gsz1);
}

typedef struct GetPartitionSplitsArgs {
    uint32_t wLocalSize;
} GetPartitionSplitsArgs;
UKL_REFLECT_STRUCT(GetPartitionSplitsArgs, wLocalSize)


// this expects [bucket, feature, partition]
DEFINE_KERNEL2(randomforest_kernels, getPartitionSplitsKernel,
               CONSTBUFFER, TreeTrainingInfo,        treeTrainingInfo,          ,
               ARGS,     GetPartitionSplitsArgs, args,                      ,
               ROBUFFER, uint32_t,               bucketNumbers,             "[nf]",
               ROBUFFER, uint32_t,               activeFeatureList,         "[naf]",
               ROBUFFER, uint32_t,               featureIsOrdinal,          "[nf]",
               ROBUFFER, W,                      buckets,                   "[np x totalBuckets]",
               ROBUFFER, W,                      wAll,                      "[np]",
               RWBUFFER, PartitionSplit,         featurePartitionSplitsOut, "[np x nf]",
               ROBUFFER, TreeDepthInfo,          treeDepthInfo,             "[1]",
               RWLOCAL,  WIndexed,               wLocal,                    "[wLocalSize]",
               SHARED,   WIndexed,               wStartBest,                2,
               SHARED,   WIndexed,               wRanked,                   MAX_CATEGORICAL_BUCKETS,
               SHARED,   uint16_t,               ranked,                    MAX_CATEGORICAL_BUCKETS,
               GID1,     uint32_t,               gid1,                      ,
               GSZ1,     uint32_t,               gsz1,                      ,
               LID0,     uint32_t,               lid0,                      ,
               LSZ0,     uint32_t,               lsz0,                      ,
               GID2,     uint32_t,               gid2,                      ,
               GSZ2,     uint32_t,               gsz2,                      )
{
    getPartitionSplitsImpl(treeTrainingInfo, bucketNumbers, activeFeatureList, featureIsOrdinal,
                           buckets, wAll, featurePartitionSplitsOut,
                           treeDepthInfo, wLocal, args.wLocalSize, wStartBest, wRanked, ranked,
                           gid1, gsz1, lid0, lsz0, gid2, gsz2);
}

// id 0: partition number
DEFINE_KERNEL2(randomforest_kernels, bestPartitionSplitKernel,
               CONSTBUFFER, TreeTrainingInfo,       treeTrainingInfo,       ,
               ROBUFFER, TreeDepthInfo,         treeDepthInfo,          "[1]",
               ROBUFFER, uint32_t,              activeFeatureList,      "[numActiveFeatures]",
               ROBUFFER, PartitionSplit,        featurePartitionSplits, "[numActivePartitions * numActiveFeatures]",
               ROBUFFER, PartitionIndex,        partitionIndexes,       "[npi]",
               RWBUFFER, IndexedPartitionSplit, allPartitionSplitsOut,  "[maxPartitions]",
               GID0,     uint32_t,              gid0,                   ,
               GSZ0,     uint32_t,              gsz0,                   )
{
    bestPartitionSplitImpl(treeTrainingInfo, treeDepthInfo, activeFeatureList,
                           featurePartitionSplits, partitionIndexes, allPartitionSplitsOut,
                           gid0, gsz0);
}

// One SIMD group. SmallSideIndex[numPartitionsOut]: used to a) know when we need to clear partitions (those on
// the small side are cleared, those on the big side are not) and b) provide a local index for partition buckets
// for the UpdateBuckets kernel:
// - 0 means it's not on the small side
// - 1-254 means we're partition number (n-1) on the small side
// - 255 means we're partition number 254 or greater on the small side
DEFINE_KERNEL2(randomforest_kernels, assignPartitionNumbersKernel,
               CONSTBUFFER, TreeTrainingInfo,                  treeTrainingInfo,             ,
               RWBUFFER, TreeDepthInfo,                    treeDepthInfo,                "[1]",
               ROBUFFER, IndexedPartitionSplit,            allPartitionSplits,           "[np]",
               RWBUFFER, PartitionIndex,                   partitionIndexesOut,          "[maxActivePartitions]",
               RWBUFFER, PartitionInfo,                    partitionInfoOut,             "[numActivePartitions]",
               RWBUFFER, uint8_t,                          smallSideIndexesOut,          "[maxActivePartitions]",
               RWBUFFER, uint16_t,                         smallSideIndexToPartitionOut, "[256]",
               SHARED,   AssignPartitionNumbersLocalState, localState,                   1,
               LANE,     uint16_t,                         lane_id,                      ,
               NLANES,   uint16_t,                         num_lanes,                    )
{
    assignPartitionNumbersImpl(treeTrainingInfo, treeDepthInfo, allPartitionSplits,
                               partitionIndexesOut, partitionInfoOut,
                               smallSideIndexesOut, smallSideIndexToPartitionOut,
                               localState, lane_id, num_lanes);
}

// [partition, bucket]
DEFINE_KERNEL2(randomforest_kernels, clearBucketsKernel,
               CONSTBUFFER, TreeTrainingInfo,  treeTrainingInfo,           ,
               ROBUFFER,    TreeDepthInfo,    treeDepthInfo,              "[1]",
               RWBUFFER,    W,                bucketsOut,                 "[numActiveBuckets * numActivePartitions]",
               RWBUFFER,    W,                wAllOut,                    "[numActivePartitions]",
               RWBUFFER,    uint32_t,         numNonZeroDirectionIndices, "[1]",
               CONSTBUFFER, uint8_t,          smallSideIndexes,           "[numActivePartitions]",
               GID0,        uint32_t,         gid0,                       ,
               GSZ0,        uint32_t,         gsz0,                       ,
               GID1,        uint32_t,         gid1,                       )
{
    clearBucketsImpl(treeTrainingInfo, treeDepthInfo, bucketsOut, wAllOut, numNonZeroDirectionIndices,
                     smallSideIndexes, gid0, gsz0, gid1);
}

// Second part: per feature plus wAll, transfer examples. [rowNumber]
DEFINE_KERNEL2(randomforest_kernels, updatePartitionNumbersKernel,
               CONSTBUFFER, TreeTrainingInfo,                  treeTrainingInfo,           ,
               ROBUFFER,    TreeDepthInfo,                    treeDepthInfo,              "[1]",
               RWBUFFER,    RowPartitionInfo,                 partitions,                 "[numRows]",
               RWBUFFER,    uint32_t,                         directions,                 "[numRows / 32]",
               RWBUFFER,    uint32_t,                         numNonZeroDirectionIndices, "[1]",
               RWBUFFER,    UpdateWorkEntry,                  nonZeroDirectionIndices,    "[numRows]",
               ROBUFFER,    uint8_t,                          smallSideIndexes,           "[numActivePartitions]",
               ROBUFFER,    IndexedPartitionSplit,            allPartitionSplits,         "[np]",
               ROBUFFER,    PartitionInfo,                    partitionInfo,              "[np]",
               ROBUFFER,    uint32_t,                         bucketData,                 "[bucketDataLength]",
               CONSTBUFFER, uint32_t,                         bucketDataOffsets,          "[nf + 1]",
               CONSTBUFFER, uint32_t,                         bucketNumbers,              "[nf + 1]",
               CONSTBUFFER, uint32_t,                         bucketEntryBits,            "[nf]",
               CONSTBUFFER, uint32_t,                         featureIsOrdinal,           "[nf]",
               ROBUFFER,    float,                            decodedRows,                "[numRows * 2]",
               SHARED,      UpdatePartitionNumbersLocalState, localState,                 1,
               GID0,        uint32_t,                         gid0,                       ,
               GSZ0,        uint32_t,                         gsz0,                       ,
               LID0,        uint32_t,                         lid0,                       ,
               LSZ0,        uint32_t,                         lsz0,                       )
{
    updatePartitionNumbersImpl(treeTrainingInfo, treeDepthInfo, partitions, directions,
                               numNonZeroDirectionIndices, nonZeroDirectionIndices, smallSideIndexes,
                               allPartitionSplits, partitionInfo,
                               bucketData, bucketDataOffsets, bucketNumbers, bucketEntryBits,
                               featureIsOrdinal, decodedRows,
                               localState, gid0, gsz0, lid0, lsz0);
}

typedef struct UpdateBucketsArgs {
    uint32_t maxLocalBuckets;
} UpdateBucketsArgs;
UKL_REFLECT_STRUCT(UpdateBucketsArgs, maxLocalBuckets)


// [row, feature]
DEFINE_KERNEL2(randomforest_kernels, updateBucketsKernel,
               ARGS,        UpdateBucketsArgs,       args,                       ,
               CONSTBUFFER, TreeTrainingInfo,         treeTrainingInfo,           ,
               ROBUFFER,    TreeDepthInfo,           treeDepthInfo,              "[1]",
               ROBUFFER,    RowPartitionInfo,        partitions,                 "[numRows]",
               RWBUFFER,    uint32_t,                directions,                 "[numRows / 32]",
               ROBUFFER,    uint32_t,                numNonZeroDirectionIndices, "[1]",
               RWBUFFER,    UpdateWorkEntry,         nonZeroDirectionIndices,    "[numRows]",
               RWBUFFER,    W,                       buckets,                    "[numActiveBuckets * numActivePartitions]",
               RWBUFFER,    W,                       wAll,                       "[numActivePartitions]",
               CONSTBUFFER, uint8_t,                 smallSideIndexes,           "[numActivePartitions]",
               CONSTBUFFER, uint16_t,                smallSideIndexToPartition,  "[256]",
               ROBUFFER,    float,                   decodedRows,                "[numRows * 2]",
               ROBUFFER,    uint32_t,                bucketData,                 "[bucketDataLength]",
               ROBUFFER,    uint32_t,                bucketDataOffsets,          "[nf + 1]",
               ROBUFFER,    uint32_t,                bucketNumbers,              "[nf + 1]",
               ROBUFFER,    uint32_t,                bucketEntryBits,            "[nf]",
               ROBUFFER,    uint32_t,                activeFeatureList,          "[naf]",
               ROBUFFER,    uint32_t,                featureIsOrdinal,           "[nf]",
               RWLOCAL,     W,                       wLocal,                     "[maxLocalBuckets]",
               SHARED,      UpdateBucketsLocalState, localState,                 1,
               GID0,        uint32_t,                gid0,                       ,
               GSZ0,        uint32_t,                gsz0,                       ,
               GID1,        uint32_t,                gid1,                       ,
               LID0,        uint32_t,                lid0,                       ,
               LSZ0,        uint32_t,                lsz0,                       )
{
    updateBucketsImpl(treeTrainingInfo, treeDepthInfo, partitions, directions,
                      numNonZeroDirectionIndices, nonZeroDirectionIndices, buckets, wAll,
                      smallSideIndexes, smallSideIndexToPartition,
                      decodedRows, bucketData, bucketDataOffsets, bucketNumbers, bucketEntryBits,
                      activeFeatureList, featureIsOrdinal,
                      wLocal, args.maxLocalBuckets, localState,
                      gid1, lid0, lsz0, gid0, gsz0);
}

// For each partition and each bucket, we up to now accumulated just the weight that needs to be transferred in
// the right hand side of the partition splits. We need to fix this up by:
// 1.  Subtracting this weight from the left hand side
// 2.  Flipping the buckets where the big amount of weight belongs on the right not on the left.
// Dimension 0 = partition number (from 0 to the old number of partitions)
// Dimension 1 = bucket number (from 0 to the number of active buckets); may be padded
DEFINE_KERNEL2(randomforest_kernels, fixupBucketsKernel,
               CONSTBUFFER, TreeTrainingInfo,  treeTrainingInfo, ,
               ROBUFFER, TreeDepthInfo,    treeDepthInfo,    "[1]",
               RWBUFFER, W,                buckets,          "[numActiveBuckets * numActivePartitions]",
               RWBUFFER, W,                wAll,             "[numActivePartitions]",
               ROBUFFER, PartitionInfo,    partitionInfo,    "[np]",
               ROBUFFER, uint8_t,          smallSideIndexes, "[numActivePartitions]",
               GID0,     uint32_t,         gid0,             ,
               GSZ0,     uint32_t,         gsz0,             ,
               GID1,     uint32_t,         gid1,             )
{
    fixupBucketsImpl(treeTrainingInfo, treeDepthInfo, buckets, wAll, partitionInfo, smallSideIndexes,
                     gid1, gid0, gsz0);
}

typedef struct PredictForestArgs {
    uint32_t numItems;
    uint32_t numForests;
    uint32_t maxDepth;
    uint32_t lanesPerRow;
    uint32_t columnStride;
} PredictForestArgs;
UKL_REFLECT_STRUCT(PredictForestArgs, numItems, numForests, maxDepth, lanesPerRow, columnStride)


// [row group]
DEFINE_KERNEL2(randomforest_kernels, predictForestKernel,
               ARGS,     PredictForestArgs, args,            ,
               ROBUFFER, uint32_t,          bucketData,      "[bucketDataLength]",
               ROBUFFER, PredictNode,       nodes,           "[numNodes]",
               ROBUFFER, uint32_t,          subsets,         "[numSubsetWords]",
               ROBUFFER, uint32_t,          forestTreeStart, "[numForests + 1]",
               ROBUFFER, int32_t,           treeRoots,       "[numTrees]",
               ROBUFFER, float,             treeWeights,     "[numTrees]",
               ROBUFFER, float,             leafPreds,       "[numLeafPreds]",
               RWBUFFER, float,             predictionsOut,  "[numItems * 2]",
               RWLOCAL,  float,             partials,        "[threadsPerGroup * 2]",
               TGID0,    uint32_t,          group,           ,
               LID0,     uint32_t,          tid,             ,
               LSZ0,     uint32_t,          ntid,            )
{
    predictForestImpl(args.numItems, args.numForests, args.maxDepth, args.lanesPerRow, args.columnStride,
                      bucketData, nodes, subsets, forestTreeStart, treeRoots, treeWeights, leafPreds,
                      predictionsOut, partials, group, tid, ntid);
}

typedef struct NumericBucketsArgs {
    uint32_t numRows;
    uint32_t numSplits;
    uint32_t numericOffset;
} NumericBucketsArgs;
UKL_REFLECT_STRUCT(NumericBucketsArgs, numRows, numSplits, numericOffset)


// [row]
DEFINE_KERNEL2(randomforest_kernels, numericBucketsKernel,
               ARGS,     NumericBucketsArgs, args,       ,
               ROBUFFER, int64_t,            keys,       "[numRows]",
               ROBUFFER, int64_t,            splitKeys,  "[numSplits]",
               RWBUFFER, uint32_t,           bucketsOut, "[numRows]",
               GID0,     uint32_t,           id,         )
{
    numericBucketsImpl(args.numRows, keys, splitKeys, args.numSplits, args.numericOffset, bucketsOut, id);
}

typedef struct CodesArgs {
    uint32_t numRows;
    uint32_t numCodes;
} CodesArgs;
UKL_REFLECT_STRUCT(CodesArgs, numRows, numCodes)


// [row]
DEFINE_KERNEL2(randomforest_kernels, markCodesKernel,
               ARGS,     CodesArgs, args,       ,
               ROBUFFER, uint32_t,  codes,      "[numRows]",
               RWBUFFER, uint32_t,  presentOut, "[numCodes / 32]",
               GID0,     uint32_t,  id,         )
{
    markCodesImpl(args.numRows, args.numCodes, codes, presentOut, id);
}

// [row]
DEFINE_KERNEL2(randomforest_kernels, categoricalBucketsKernel,
               ARGS,     CodesArgs, args,        ,
               ROBUFFER, uint32_t,  codes,       "[numRows]",
               ROBUFFER, uint32_t,  codeBuckets, "[numCodes]",
               RWBUFFER, uint32_t,  bucketsOut,  "[numRows]",
               GID0,     uint32_t,  id,          )
{
    categoricalBucketsImpl(args.numRows, args.numCodes, codes, codeBuckets, bucketsOut, id);
}

typedef struct RowBucketsArgs {
    uint32_t numRows;
    uint32_t numColumns;
    uint32_t columnStride;
} RowBucketsArgs;
UKL_REFLECT_STRUCT(RowBucketsArgs, numRows, numColumns, columnStride)


// [column * columnStride + word]
DEFINE_KERNEL2(randomforest_kernels, rowBucketsKernel,
               ARGS,     RowBucketsArgs, args,          ,
               ROBUFFER, uint32_t,       columns,       "[numColumns * ROW_BUCKET_WORDS]",
               ROBUFFER, uint64_t,       numbers,       "[numRows * numNumbers]",
               ROBUFFER, uint32_t,       codes,         "[numRows * numCodes]",
               ROBUFFER, int64_t,        splitKeys,     "[numSplitKeys]",
               ROBUFFER, uint32_t,       codeTables,    "[numCodeTables]",
               RWBUFFER, uint32_t,       bucketDataOut, "[numColumns * columnStride]",
               GID0,     uint32_t,       id,            )
{
    rowBucketsImpl(args.numRows, args.numColumns, args.columnStride, columns, numbers, codes, splitKeys,
                   codeTables, bucketDataOut, id);
}

typedef struct PackBucketsArgs {
    uint32_t numRows;
    uint32_t entryBits;
    uint32_t numWords;
} PackBucketsArgs;
UKL_REFLECT_STRUCT(PackBucketsArgs, numRows, entryBits, numWords)


// [word]
DEFINE_KERNEL2(randomforest_kernels, packBucketsKernel,
               ARGS,     PackBucketsArgs, args,     ,
               ROBUFFER, uint32_t,        buckets,  "[numRows]",
               RWBUFFER, uint32_t,        wordsOut, "[numWords]",
               GID0,     uint32_t,        id,       )
{
    packBucketsImpl(args.numRows, args.entryBits, args.numWords, buckets, wordsOut, id);
}

typedef struct BagRowsArgs {
    uint32_t numExamples;
    uint32_t seed;
    uint32_t keep24;
    uint32_t sample;
    float scale;
} BagRowsArgs;
UKL_REFLECT_STRUCT(BagRowsArgs, numExamples, seed, keep24, sample, scale)


// [example]
DEFINE_KERNEL2(randomforest_kernels, bagRowsKernel,
               ARGS,     BagRowsArgs, args,           ,
               ROBUFFER, float,       baseRows,       "[numExamples * 2]",
               RWBUFFER, float,       decodedRowsOut, "[numExamples * 2]",
               GID0,     uint32_t,    id,             )
{
    bagRowsImpl(args.numExamples, args.seed, args.keep24, args.sample, args.scale, baseRows, decodedRowsOut, id);
}

// [thread]
DEFINE_KERNEL2(randomforest_kernels, rootWeightsKernel,
               ROBUFFER, uint32_t, bucketNumbers,     "[nf + 1]",
               ROBUFFER, uint32_t, activeFeatureList, "[naf]",
               ROBUFFER, W,        partitionBuckets,  "[numBuckets]",
               RWBUFFER, W,        wAllOut,           "[naf]",
               GID0,     uint32_t, id,                ,
               GSZ0,     uint32_t, n,                 )
{
    rootWeightsImpl(bucketNumbers, activeFeatureList, partitionBuckets, wAllOut, id, n);
}
