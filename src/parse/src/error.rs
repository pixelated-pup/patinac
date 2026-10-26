use errors::{Diagnostic, Report};

use crate::TokKind;

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum ErrorKind {
    BadToken,
    Unexpected(TokKind, Option<&'static str>),
    Mismatched { expected: TokKind, found: TokKind },
    //#[display("`self` must be the first parameter")]
    SelfNotFirst,
    BadUnicodeEscape,
    //#[display("only type and def items can be public")]
    BadPub,
    NotDefInImpl,
}

impl Diagnostic for ErrorKind {
    type Ctx<'a> = ();

    fn report(self, (): ()) -> Report {
        match self {
            Self::BadToken => Report::error("invalid token"),
            Self::Unexpected(found, msg) => {
                let report = Report::error("unexpected token").with_label(format!("found {found}"));
                match msg {
                    Some(msg) => report.with_note(msg),
                    None => report,
                }
            }
            Self::Mismatched { expected, found } => Report::error("unexpected token")
                .with_label(format!("expected {expected}, found {found}")),
            Self::SelfNotFirst => todo!(),
            Self::BadUnicodeEscape => Report::error("invalid unicode escape").with_note(
                "unicode escapes must be less than 0x110000 and not between 0xD800 and 0xE000",
            ),
            Self::BadPub => todo!(),
            Self::NotDefInImpl => Report::error("impl blocks can only contain definition items"),
        }
    }
}
