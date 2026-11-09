//! Explicit signing utility for configuration maintenance. It is not a query function.
//! Requests contain messages and environment-variable names, never private-key values.
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    id: String,
    key_env: String,
    message: String,
}

#[derive(Serialize, Debug, PartialEq, Eq)]
struct Reply {
    id: String,
    status: String,
    signature: String,
    detail: String,
}

fn run_with(_requests: Vec<Request>, _environment: impl FnMut(&str) -> Option<String>) -> Vec<Reply> {
    Vec::new()
}

pub(super) fn run(text: &str) -> Result<String, String> {
    let requests: Vec<Request> = serde_json::from_str(text).map_err(|e| e.to_string())?;
    serde_json::to_string(&run_with(requests, |name| std::env::var_os(name).map(|v| v.to_string_lossy().into_owned())))
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request(id: &str, key: &str, message: &str) -> Request {
        Request { id: id.into(), key_env: key.into(), message: message.into() }
    }
    #[test]
    fn batch_signing_preserves_requests_and_reads_each_key_once() {
        // RFC 8032 test vector 1, a public test seed, never a repository key.
        let seed = "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60";
        let mut reads = 0;
        let rows = run_with(vec![request("a", "fixture", ""), request("b", "fixture", "")], |name| {
            assert_eq!(name,"fixture"); reads += 1; Some(seed.into())
        });
        assert_eq!(reads,1);
        assert_eq!(rows.len(),2);
        assert_eq!(rows[0].id,"a");assert_eq!(rows[1].id,"b");
        assert_eq!(rows[0].status,"signed");
        assert_eq!(rows[0].signature,"5VZDAMNgrHKQ4IbigG6CgqhId/Hrjl2XTYc+BlIkFUVaPUi/9J84kdY+76ZhH66HHO9XwOtJWuJGVVFGq5KBAA==");
        assert_eq!(rows[0].signature,rows[1].signature);
    }
    #[test]
    fn one_bad_key_does_not_poison_an_independent_request() {
        let rows=run_with(vec![request("missing","absent","x"),request("bad","invalid","x"),request("length","short","x"),request("ok","valid","x")], |name| match name {
            "absent"=>None,"invalid"=>Some("0g".into()),"short"=>Some("00".into()),"valid"=>Some("00".repeat(32)),_=>unreachable!()
        });
        assert_eq!(rows.iter().map(|r|r.status.as_str()).collect::<Vec<_>>(),["missing-key","invalid-hex","key-length","signed"]);
        assert_eq!(rows[1].detail,"non-hexadecimal number found in fromhex() arg at position 1");
        assert!(rows[..3].iter().all(|r|r.signature.is_empty()));
    }
    #[test]
    fn whitespace_belongs_between_bytes_and_errors_keep_character_positions() {
        for (key,position) in [("0 0",1),("00x",3),("00💥",2),("00\u{00a0}00",2)] {
            let rows=run_with(vec![request("test","fixture","")], |_|Some(key.into()));
            assert_eq!(rows[0].status,"invalid-hex");
            assert_eq!(rows[0].detail,format!("non-hexadecimal number found in fromhex() arg at position {position}"));
        }
        let rows=run_with(vec![request("test","fixture","")], |_|Some(format!(" \t{}\n", "00 ".repeat(32))));
        assert_eq!(rows[0].status,"signed");
    }
}
