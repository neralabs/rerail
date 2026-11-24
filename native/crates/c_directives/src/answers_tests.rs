//! The grammars' shipped answers (decision 0669): each file answers every question its grammar's
//! compiles ask, C's and C++'s (`answers::check_shipped`; `RERAIL_BLESS_ANSWERS=1` writes them again).

use rerail_compiler::answers::check_shipped;
use std::path::PathBuf;

fn path(p: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(p)
}

#[test]
fn the_directives_answers_answer_the_grammar() {
    check_shipped(&path("directives.answers"), crate::ANSWERS, |known| Ok(crate::lower_known(crate::GRAMMAR, crate::KINDS, crate::FIELDS, known)?.1));
}

#[test]
fn the_preprocessor_answers_answer_its_grammars() {
    check_shipped(&path("bitlex_pp/pp.answers"), crate::pp::ANSWERS, |known| {
        let mut made = Vec::new();
        for cpp in [false, true] {
            made.extend(crate::lower_known(&crate::pp::grammar(cpp), rerail_c_pp::tape::KINDS, rerail_c_pp::tape::FIELDS, known.clone())?.1);
        }
        Ok(made)
    });
}

#[test]
fn the_tokens_answers_answer_their_grammars() {
    check_shipped(&path("bitlex_tokens/tokens.answers"), crate::tokens::ANSWERS, |known| {
        let mut made = Vec::new();
        for cpp in [false, true] {
            made.extend(crate::lower_known(&crate::tokens::grammar(cpp), crate::tokens::KINDS, &[], known.clone())?.1);
        }
        Ok(made)
    });
}
