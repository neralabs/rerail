//! Explicit signing utility for configuration maintenance. It is not a query function.
//! Requests contain messages and environment-variable names, never private-key values.
use serde::{Deserialize, Serialize};
use base64::{Engine, engine::general_purpose::STANDARD};
use ed25519_dalek::{Signer, SigningKey};
use std::collections::HashMap;

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

enum Key {
    Ready(SigningKey),
    Failed(&'static str, String),
}

fn decode_key(text: Option<String>) -> Key {
    let Some(text)=text.filter(|s| !s.is_empty()) else { return Key::Failed("missing-key",String::new()) };
    let invalid=|position| Key::Failed("invalid-hex",format!("non-hexadecimal number found in fromhex() arg at position {position}"));
    // CPython rejects the first non-ASCII codepoint before decoding ASCII pairs.
    if let Some(position)=text.chars().position(|c| !c.is_ascii()) { return invalid(position); }
    let digit=|c| match c { b'0'..=b'9'=>Some(c-b'0'),b'a'..=b'f'=>Some(c-b'a'+10),b'A'..=b'F'=>Some(c-b'A'+10),_=>None };
    let input=text.as_bytes();let mut bytes=[0;32];let (mut i,mut count)=(0,0);
    while i<input.len() {
        if matches!(input[i],9..=13|32) { i+=1;continue; }
        let Some(high)=digit(input[i]) else { return invalid(i) };
        let Some(low)=input.get(i+1).copied().and_then(digit) else { return invalid(i+1) };
        if count<32 { bytes[count]=high*16+low; }
        count+=1;i+=2;
    }
    if count!=32 { return Key::Failed("key-length",String::new()); }
    Key::Ready(SigningKey::from_bytes(&bytes))
}

fn run_with(requests: Vec<Request>, mut environment: impl FnMut(&str) -> Option<String>) -> Vec<Reply> {
    let mut keys=HashMap::new();
    requests.into_iter().map(|request| {
        let key=keys.entry(request.key_env.clone()).or_insert_with(|| decode_key(environment(&request.key_env)));
        match key {
            Key::Ready(key)=>Reply { id:request.id,status:"signed".into(),signature:STANDARD.encode(key.sign(request.message.as_bytes()).to_bytes()),detail:String::new() },
            Key::Failed(status,detail)=>Reply { id:request.id,status:(*status).into(),signature:String::new(),detail:detail.clone() },
        }
    }).collect()
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
        assert_eq!(rows[0].signature,"5VZDAMNgrHKQhuLMgG6CioSHfx645dl02HPgZSJJAVVfuIIVkKM7rMYeOXAc+bRr0lv18FlbviRlUUFDjnoQCw==");
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
        for (key,position) in [("0 0",1),("00x",2),("00💥",2),("00\u{00a0}00",2),("g💥",1),("0\u{b}0",1)] {
            let rows=run_with(vec![request("test","fixture","")], |_|Some(key.into()));
            assert_eq!(rows[0].status,"invalid-hex");
            assert_eq!(rows[0].detail,format!("non-hexadecimal number found in fromhex() arg at position {position}"));
        }
        let rows=run_with(vec![request("test","fixture","")], |_|Some(format!(" \t{}\n", "00 ".repeat(32))));
        assert_eq!(rows[0].status,"signed");
    }
}
