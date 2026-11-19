/** randomforest_procedure.cc                                             -*- C++ -*-
    Mathieu Marquis Bolduc, 11 Mars 2016
    Copyright (c) 2016 mldb.ai inc.  All rights reserved.

    This file is part of MLDB. Copyright 2016 mldb.ai inc. All rights reserved.

    Procedure to train a random forest binary classifier.
*/

#include "randomforest_procedure.h"
#include "mldb/arch/bit_range_ops.h"
#include "mldb/arch/bitops.h"
#include "mldb/arch/timers.h"
#include "mldb/utils/profile.h"
#include "mldb/plugins/jml/randomforest.h"
#include "mldb/plugins/jml/value_descriptions.h"
#include "mldb/builtin/sql_expression_extractors.h"
#include "mldb/plugins/jml/classifier.h"
#include "mldb/rest/in_process_rest_connection.h"
#include "mldb/core/bound_queries.h"
#include "mldb/core/mldb_engine.h"
#include "mldb/types/any_impl.h"
#include "mldb/types/basic_value_descriptions.h"
#include "mldb/types/set_description.h"
#include "mldb/vfs/fs_utils.h"
#include "mldb/builtin/sql_config_validator.h"
#include "mldb/arch/simd_vector.h"
#include "mldb/utils/log.h"
#include "mldb/plugins/jml/jml/decision_tree.h"
#include "mldb/plugins/jml/jml/committee.h"
#include "mldb/engine/column_scope.h"
#include "mldb/base/parallel.h"


#include <random>

using namespace std;
using namespace ML;


namespace MLDB {

using namespace RF;


DEFINE_STRUCTURE_DESCRIPTION(RandomForestProcedureConfig);

RandomForestProcedureConfigDescription::
RandomForestProcedureConfigDescription()
{
    addField("trainingData", &RandomForestProcedureConfig::trainingData,
             "Specification of the data for input to the classifier procedure. "
             "The select expression must contain these two sub-expressions: one row expression "
             "to identify the features on which to train and one scalar expression "
             "to identify the label.  The type of the label expression must be a boolean (0 or 1)"
             "Labels with a null value will have their row skipped. "
             "The select statement does not support groupby and having clauses. "
             "Also, unlike most select expressions, this one can only select whole columns, "
             "not expressions involving columns. So X will work, but not X + 1. "
             "If you need derived values in the select expression, create a dataset with "
             "the derived columns as a previous step and run the classifier over that dataset instead.");
    addField("modelFileUrl", &RandomForestProcedureConfig::modelFileUrl,
             "URL where the model file (with extension '.cls') should be saved. "
             "This file can be loaded by the ![](%%doclink classifier function). ");
    addField("featureVectorSamplings", &RandomForestProcedureConfig::featureVectorSamplings,
             "Number of samplings of feature vectors. "
             "The total number of bags will be featureVectorSamplings*featureSamplings.", 5);
    addField("featureVectorSamplingProp", &RandomForestProcedureConfig::featureVectorSamplingProp,
             "Proportion of feature vectors to select in each sample. ", 0.3f);
    addField("featureSamplings", &RandomForestProcedureConfig::featureSamplings,
             "Number of samplings of features. "
             "The total number of bags will be ```featureVectorSamplings```*```featureSamplings```.", 20);
    addField("featureSamplingProp", &RandomForestProcedureConfig::featureSamplingProp,
             "Proportion of features to select in each sample. ", 0.3f);
    addField("maxDepth", &RandomForestProcedureConfig::maxDepth,
             "Maximum depth of the trees ", 20);
    addField("functionName", &RandomForestProcedureConfig::functionName,
             "If specified, an instance of the ![](%%doclink classifier function) of this name will be created using "
             "the trained model. Note that to use this parameter, the `modelFileUrl` must "
             "also be provided.");
    addField("verbosity", &RandomForestProcedureConfig::verbosity,
             "Should the procedure be verbose for debugging and tuning purposes", false);
    addField("sampleFeatureVectors", &RandomForestProcedureConfig::sampleFeatureVectors,
             "Should we sample feature vectors (default yes)?  Should only be set "
             "to false for testing or with a very small number of featureVectorSamplings.",
             true);
    addParent<ProcedureConfig>();

    onPostValidate = chain(validateQuery(&RandomForestProcedureConfig::trainingData,
                                         NoGroupByHaving(),
                                         PlainColumnSelect(),
                                         MustContainFrom(),
                                         FeaturesLabelSelect()),
                           validateFunction<RandomForestProcedureConfig>());
}

/*****************************************************************************/
/* CLASSIFIER PROCEDURE                                                       */
/*****************************************************************************/

RandomForestProcedure::
RandomForestProcedure(MldbEngine * owner,
            PolyConfig config,
            const std::function<bool (const Json::Value &)> & onProgress)
    : Procedure(owner)
{
    this->procedureConfig = config.params.convert<RandomForestProcedureConfig>();
}

Any
RandomForestProcedure::
getStatus() const
{
    return Any();
}

// RandomForestRNG: in randomforest_procedure.rs (random_forest_rng).

RunOutput
RandomForestProcedure::
run(const ProcedureRunConfig & run,
    const std::function<bool (const Json::Value &)> & onProgress) const
{
    //Todo: we will need heuristics for those. (MLDB-1449)
    int maxBagsAtOnce = 8;
    //int maxTreesAtOnce = 20;

    RandomForestProcedureConfig runProcConf =
        applyRunConfOverProcConf(procedureConfig, run);

    Timer timer;

    // this includes being empty
    if(!runProcConf.modelFileUrl.valid()) {
         throw MLDB::Exception("modelFileUrl is not valid");
    }

    checkWritability(runProcConf.modelFileUrl.toDecodedString(),
                     "modelFileUrl");

    // 1.  Get the input dataset
    SqlExpressionMldbScope context(engine);

    ConvertProgressToJson convertProgressToJson(onProgress);
    auto boundDataset = runProcConf.trainingData.stm->from->bind(context, convertProgressToJson);

    ML::Mutable_Feature_Info labelInfo = ML::Mutable_Feature_Info(ML::BOOLEAN);
    labelInfo.set_biased(true);

    auto extractWithinExpression = [](std::shared_ptr<SqlExpression> expr)
        -> std::shared_ptr<SqlRowExpression>
        {
            auto withinExpression = std::dynamic_pointer_cast<const SelectWithinExpression>(expr);
            if (withinExpression)
                return withinExpression->select;

            return nullptr;
        };

    auto labelVal = extractNamedSubSelect("label", runProcConf.trainingData.stm->select);
    auto featuresVal = extractNamedSubSelect("features", runProcConf.trainingData.stm->select);
    if (!labelVal || !featuresVal) {
        throw AnnotatedException(400, "trainingData must return a 'features' row and a 'label'");
    }

    auto weightVal = extractNamedSubSelect("weight", runProcConf.trainingData.stm->select);
    auto weight = weightVal ? weightVal->expression : SqlExpression::ONE;

    shared_ptr<SqlRowExpression> subSelect = extractWithinExpression(featuresVal->expression);

    if (!subSelect) {
        throw AnnotatedException(400, "trainingData must return a 'features' row");
    }

    auto label = labelVal->expression;
    
    ColumnScope colScope(engine, boundDataset.dataset);
    auto boundLabel = label->bind(colScope);
    auto boundWhere = runProcConf.trainingData.stm->where->bind(colScope);
    auto boundWeight = weight->bind(colScope);

    INFO_MSG(logger) << "label " << label->surface << " uses columns " << jsonEncode(colScope.requiredColumns);

    Timer labelsTimer;

    std::vector<std::vector<float> > labelsWhereWeight
        = colScope.runFloat({boundLabel, boundWhere, boundWeight});

    std::vector<float> & labels = labelsWhereWeight[0];
    std::vector<float> & wheres = labelsWhereWeight[1];
    std::vector<float> & weights = labelsWhereWeight[2];

    INFO_MSG(logger) << "got " << labels.size() << " labels in " << labelsTimer.elapsed();

    // keepExample, and the count of the rows kept: in randomforest_procedure.rs (training_data).

    SelectExpression select({subSelect});

    auto getColumnsInExpression = [&] (const SqlExpression & expr)
        -> std::set<ColumnPath>
        {
            std::set<ColumnPath> knownInputColumns;

            // Find only those variables used
            SqlExpressionDatasetScope scope(boundDataset);
            
            auto selectBound = select.bind(scope);

            for (auto & c : selectBound.info->getKnownColumns()) {
                knownInputColumns.insert(c.columnName);
            }

            return knownInputColumns;
        };

    std::set<ColumnPath> knownInputColumns
        = getColumnsInExpression(select);

    auto featureSpace = std::make_shared<DatasetFeatureSpace>
        (boundDataset.dataset, labelInfo, knownInputColumns, true /* bucketize */);

    INFO_MSG(logger) << "feature space construction took " << timer.elapsed();
    timer.restart();

    auto outputFeature = [&]() {
        stringstream str;
        for (auto& c : knownInputColumns) {
            str << c << " feature " << featureSpace->getFeature(c)
                << " had " << featureSpace->columnInfo[c].buckets.numBuckets
                << " buckets"
                << " type is " << featureSpace->columnInfo[c].info << endl;
        }
        return str.str();
    };
    
    TRACE_MSG(logger) << outputFeature();

    // Get the feature buckets per row

    //TODO: Need to pack this into 1 memory buffer

    int numFeatures = knownInputColumns.size();
    INFO_MSG(logger) << "NUM FEATURES : " << numFeatures;

    // The training rows (allData), the bags (doFeatureVectorSampling: the bootstrap, reweightAndCompact, the
    // feature samplings, trainMultipleSamplings) and the trees in order: in randomforest_procedure.rs
    // (training_data, bag_counts, bag, train_forest).

    cerr << "tree construction took " << beforeTrees.elapsed() << endl;
    
    shared_ptr<Committee> result = make_shared<Committee>(contFeatureSpace, labelFeature);

    for (unsigned i = 0;  i < totalResultCount;  ++i)
        result->add(results[i], 1.0 / totalResultCount);

    ML::Classifier classifier(result);

    //Save the model, create the function

    bool saved = true;
    try {
        makeUriDirectory(
            runProcConf.modelFileUrl.toDecodedString());
        classifier.save(runProcConf.modelFileUrl.toString());
    }
    catch (const std::exception & exc) {
        saved = false;
        INFO_MSG(logger) << "Error saving classifier: " << exc.what();
    }

    if(saved && !runProcConf.functionName.empty()) {
        PolyConfig clsFuncPC;
        clsFuncPC.type = "classifier";
        clsFuncPC.id = runProcConf.functionName;
        clsFuncPC.params = ClassifyFunctionConfig(runProcConf.modelFileUrl);

        obtainFunction(engine, clsFuncPC, onProgress);
    }

    return RunOutput();
}

namespace{
	static RegisterProcedureType<RandomForestProcedure, RandomForestProcedureConfig>
	regPrototypeClassifier(builtinPackage(),
	              "Train a supervised binary random forest",
	              "procedures/RandomForest.md.html");

}

} // namespace MLDB
