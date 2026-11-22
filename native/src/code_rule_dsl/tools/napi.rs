//! The engine's half of the CLI's tools (`rerail_boot::code_rule_dsl::tools`): the host's napi wrapper, and the
//! entry points that boot the program before a command, where nothing booted it as the library loaded (the CLI
//! binary, the browser's build).

use rerail_boot::code_rule_dsl::tools as boot;

/// The package CLI's SQL tool, sharing the standalone command's request and execution path.
#[napi_derive::napi(catch_unwind)]
pub fn execute_sql_request(store_path: String, request_json: String) -> napi::Result<String> {
    boot::sql(std::path::Path::new(&store_path), &request_json).map_err(napi::Error::from_reason)
}

/// `tools::cli`, after the program is booted.
pub fn cli(args: &[String], machine: &std::sync::Arc<crate::threads::Machine>) -> (Result<String, String>, Option<String>) {
    crate::install_store_functions();
    boot::cli(args, machine)
}

/// `tools::cli_in`, after the program is booted.
pub fn cli_in(
    args: &[String],
    kept: &mut crate::code_rule_dsl::logic::plan::corpus::session::Kept,
    shared: &std::sync::Arc<crate::code_rule_dsl::logic::run::Shared>,
) -> (Result<String, String>, Option<String>) {
    crate::install_store_functions();
    boot::cli_in(args, kept, shared)
}
